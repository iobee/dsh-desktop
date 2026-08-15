use std::{
    collections::HashSet,
    env,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::Manager;

const PACKAGE_NAME: &str = "@deepseek-ai/dsh";
const INSTALL_SCRIPT_PACKAGES: [&str; 5] = [
    "@deepseek-ai/dsh-subprocess-local",
    "@google/genai",
    "koffi",
    "node-pty",
    "protobufjs",
];
const READY_PREFIX: &str = "dsh web: ";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const UPDATE_DELAY: Duration = Duration::from_secs(60);
const UPDATE_INTERVAL_SECS: u64 = 24 * 60 * 60;
const FAILED_RETRY_SECS: u64 = 60 * 60;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupInfo {
    pub url: String,
    pub dsh_version: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledManifest {
    node_version: String,
    npm_version: String,
    dsh_version: String,
    platform: String,
    arch: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeState {
    active: Option<String>,
    previous: Option<String>,
    pending: Option<String>,
    last_update_attempt: Option<u64>,
    last_successful_check: Option<u64>,
}

struct Paths {
    node: PathBuf,
    npm_cli: PathBuf,
    bundled_runtime: PathBuf,
    runtimes: PathBuf,
    state: PathBuf,
    logs: PathBuf,
    npm_cache: PathBuf,
    smoke: PathBuf,
}

struct ManagedProcess {
    child: Child,
    process_id: u32,
}

impl ManagedProcess {
    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn terminate(&mut self) {
        terminate_child(&mut self.child, self.process_id);
    }
}

pub struct RuntimeManager {
    paths: Paths,
    manifest: BundledManifest,
    state: Mutex<RuntimeState>,
    process: Mutex<Option<ManagedProcess>>,
    startup: Mutex<Option<StartupInfo>>,
    start_gate: Mutex<()>,
    update_scheduled: AtomicBool,
    update_running: AtomicBool,
    shutting_down: AtomicBool,
}

impl RuntimeManager {
    pub fn new(
        resource_dir: PathBuf,
        app_data_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let manifest_path = resolve_resource(&resource_dir, "runtime-manifest.json");
        let manifest: BundledManifest =
            serde_json::from_reader(File::open(&manifest_path).map_err(|error| {
                format!(
                    "cannot open bundled runtime manifest {}: {error}",
                    manifest_path.display()
                )
            })?)?;
        Version::parse(&manifest.dsh_version)?;
        let current_platform = node_platform_name(env::consts::OS);
        let current_arch = node_arch_name(env::consts::ARCH);
        if manifest.platform != current_platform || manifest.arch != current_arch {
            return Err(format!(
                "bundled runtime targets {}-{}, but this computer is {}-{}",
                manifest.platform, manifest.arch, current_platform, current_arch
            )
            .into());
        }

        let data_root = app_data_dir.join("runtime");
        let logs = app_data_dir.join("logs");
        fs::create_dir_all(data_root.join("versions"))?;
        fs::create_dir_all(&logs)?;
        fs::create_dir_all(data_root.join("npm-cache"))?;
        fs::create_dir_all(data_root.join("smoke"))?;

        let paths = Paths {
            node: resolve_resource(&resource_dir, bundled_node_resource()),
            npm_cli: resolve_resource(&resource_dir, "npm/node_modules/npm/bin/npm-cli.js"),
            bundled_runtime: resolve_resource(&resource_dir, "bootstrap-runtime"),
            runtimes: data_root.join("versions"),
            state: data_root.join("state.json"),
            logs,
            npm_cache: data_root.join("npm-cache"),
            smoke: data_root.join("smoke"),
        };
        for required in [&paths.node, &paths.npm_cli, &paths.bundled_runtime] {
            if !required.exists() {
                return Err(format!(
                    "bundled runtime resource is missing: {}",
                    required.display()
                )
                .into());
            }
        }

        let state = read_state(&paths.state).unwrap_or_default();
        append_log(
            &paths.logs.join("desktop.log"),
            &format!(
                "DSH Desktop initialized (Node {}, npm {}, bundled dsh {})",
                manifest.node_version, manifest.npm_version, manifest.dsh_version
            ),
        );
        Ok(Self {
            paths,
            manifest,
            state: Mutex::new(state),
            process: Mutex::new(None),
            startup: Mutex::new(None),
            start_gate: Mutex::new(()),
            update_scheduled: AtomicBool::new(false),
            update_running: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn start(&self, app: &tauri::AppHandle) -> Result<StartupInfo, String> {
        let _gate = self
            .start_gate
            .lock()
            .map_err(|_| "启动锁已损坏".to_string())?;
        self.shutting_down.store(false, Ordering::Release);

        {
            let mut process = self
                .process
                .lock()
                .map_err(|_| "进程状态锁已损坏".to_string())?;
            if process.as_mut().is_some_and(ManagedProcess::is_running) {
                if let Some(info) = self
                    .startup
                    .lock()
                    .map_err(|_| "启动状态锁已损坏".to_string())?
                    .clone()
                {
                    return Ok(info);
                }
            } else {
                process.take();
            }
        }

        let (candidates, pending) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "版本状态锁已损坏".to_string())?;
            self.reconcile_state(&mut state);
            self.save_state(&state)?;
            (
                launch_candidates(&state, &self.manifest.dsh_version),
                state.pending.clone(),
            )
        };

        let mut failures = Vec::new();
        for version in candidates {
            let Some(runtime_path) = self.runtime_path(&version) else {
                failures.push(format!("dsh {version}: 安装目录不存在"));
                continue;
            };
            match self.launch_at(&version, &runtime_path, None, START_TIMEOUT) {
                Ok((process, url)) => {
                    {
                        let mut state = self
                            .state
                            .lock()
                            .map_err(|_| "版本状态锁已损坏".to_string())?;
                        let old_active = state.active.clone();
                        if state.pending.as_deref() == Some(&version) {
                            state.previous = old_active.filter(|old| old != &version);
                            state.active = Some(version.clone());
                            state.pending = None;
                        } else if state.active.as_deref() != Some(&version) {
                            state.previous = old_active.filter(|old| old != &version);
                            state.active = Some(version.clone());
                        }
                        if pending.is_some() && pending.as_deref() != Some(&version) {
                            state.pending = None;
                        }
                        self.save_state(&state)?;
                    }
                    *self
                        .process
                        .lock()
                        .map_err(|_| "进程状态锁已损坏".to_string())? = Some(process);
                    let info = StartupInfo {
                        url,
                        dsh_version: version.clone(),
                    };
                    *self
                        .startup
                        .lock()
                        .map_err(|_| "启动状态锁已损坏".to_string())? = Some(info.clone());
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_title(&format!("DSH Desktop · dsh {version}"));
                    }
                    append_log(
                        &self.desktop_log(),
                        &format!("active runtime: dsh {version}"),
                    );
                    return Ok(info);
                }
                Err(error) => {
                    append_log(
                        &self.desktop_log(),
                        &format!("dsh {version} failed to start: {error}"),
                    );
                    failures.push(format!("dsh {version}: {error}"));
                }
            }
        }

        Err(format!("没有可启动的 dsh 版本。\n{}", failures.join("\n")))
    }

    pub fn schedule_update_check(self: &std::sync::Arc<Self>, app: tauri::AppHandle) {
        if self.update_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = std::sync::Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(UPDATE_DELAY);
            if !manager.shutting_down.load(Ordering::Acquire) {
                manager.check_for_updates(app, false);
            }
        });
    }

    pub fn check_for_updates(self: &std::sync::Arc<Self>, app: tauri::AppHandle, force: bool) {
        if self.update_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = std::sync::Arc::clone(self);
        thread::spawn(move || {
            let result = manager.check_for_updates_inner(force);
            match result {
                Ok(UpdateOutcome::Staged(version)) => {
                    append_log(
                        &manager.desktop_log(),
                        &format!("dsh {version} staged; restart to apply"),
                    );
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window
                            .set_title(&format!("DSH Desktop · dsh {version} 可在重启后使用"));
                    }
                }
                Ok(UpdateOutcome::Current) => {
                    append_log(&manager.desktop_log(), "npm latest is already active")
                }
                Ok(UpdateOutcome::Skipped) => {}
                Err(error) => append_log(
                    &manager.desktop_log(),
                    &format!("update check failed: {error}"),
                ),
            }
            manager.update_running.store(false, Ordering::Release);
        });
    }

    pub fn terminate(&self) {
        self.shutting_down.store(true, Ordering::Release);
        if let Ok(mut slot) = self.process.lock() {
            if let Some(mut process) = slot.take() {
                process.terminate();
            }
        }
        if let Ok(mut startup) = self.startup.lock() {
            startup.take();
        }
    }

    pub fn open_logs(&self) -> Result<(), String> {
        let mut command = Command::new(log_directory_opener());
        configure_background_command(&mut command);
        command
            .arg(&self.paths.logs)
            .spawn()
            .map_err(|error| format!("无法打开日志目录：{error}"))?;
        Ok(())
    }

    fn reconcile_state(&self, state: &mut RuntimeState) {
        let active_available = state
            .active
            .as_deref()
            .and_then(|version| self.runtime_path(version))
            .is_some();
        if !active_available {
            state.active = Some(self.manifest.dsh_version.clone());
            state.previous = None;
        }
        if state
            .pending
            .as_deref()
            .and_then(|version| self.runtime_path(version))
            .is_none()
        {
            state.pending = None;
        }
        if state
            .previous
            .as_deref()
            .and_then(|version| self.runtime_path(version))
            .is_none()
        {
            state.previous = None;
        }

        if state.pending.is_none()
            && state
                .active
                .as_deref()
                .is_some_and(|active| version_is_newer(&self.manifest.dsh_version, active))
        {
            state.previous = state.active.replace(self.manifest.dsh_version.clone());
        }
    }

    fn runtime_path(&self, version: &str) -> Option<PathBuf> {
        let installed = self.paths.runtimes.join(version);
        if runtime_cli(&installed).is_file() {
            Some(installed)
        } else if version == self.manifest.dsh_version
            && runtime_cli(&self.paths.bundled_runtime).is_file()
        {
            Some(self.paths.bundled_runtime.clone())
        } else {
            None
        }
    }

    fn launch_at(
        &self,
        version: &str,
        runtime_path: &Path,
        dsh_home: Option<&Path>,
        timeout: Duration,
    ) -> Result<(ManagedProcess, String), String> {
        let cli = runtime_cli(runtime_path);
        let mut command = Command::new(&self.paths.node);
        command
            .arg(cli)
            .args(["web", "--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NODE_ENV", "production")
            .env("DSH_DESKTOP", "1")
            .env("PATH", runtime_path_env(&self.paths.node, runtime_path))
            .current_dir(user_home());
        if let Some(home) = dsh_home {
            command.env("DSH_HOME", home);
        }
        configure_managed_command(&mut command);

        append_log(&self.desktop_log(), &format!("launching dsh {version}"));
        let mut child = command
            .spawn()
            .map_err(|error| format!("无法创建 Node 进程：{error}"))?;
        let process_id = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法读取 dsh 标准输出".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法读取 dsh 错误输出".to_string())?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let output_log = self.runtime_log();
        let error_log = output_log.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                append_log(&output_log, &format!("stdout | {line}"));
                if let Some(url) = parse_ready_url(&line) {
                    let _ = ready_tx.send(url);
                }
            }
        });
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                append_log(&error_log, &format!("stderr | {line}"));
            }
        });

        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(url) = ready_rx.try_recv() {
                return Ok((ManagedProcess { child, process_id }, url));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!("进程在就绪前退出（{status}）"));
                }
                Ok(None) => {}
                Err(error) => return Err(format!("无法读取进程状态：{error}")),
            }
            if Instant::now() >= deadline {
                terminate_child(&mut child, process_id);
                return Err(format!("{timeout:?} 内没有收到就绪信号"));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn check_for_updates_inner(&self, force: bool) -> Result<UpdateOutcome, String> {
        let now = unix_time();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "版本状态锁已损坏".to_string())?;
            if !force && !update_is_due(&state, now) {
                return Ok(UpdateOutcome::Skipped);
            }
            state.last_update_attempt = Some(now);
            self.save_state(&state)?;
        }

        let mut command = Command::new(&self.paths.node);
        configure_background_command(&mut command);
        let output = command
            .arg(&self.paths.npm_cli)
            .args(["view", PACKAGE_NAME, "dist-tags.latest", "--json"])
            .env("npm_config_cache", &self.paths.npm_cache)
            .env("npm_config_update_notifier", "false")
            .output()
            .map_err(|error| format!("无法执行 npm view：{error}"))?;
        if !output.status.success() {
            return Err(format!(
                "npm view 失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let latest = parse_npm_version(&output.stdout)?;
        Version::parse(&latest)
            .map_err(|error| format!("npm latest 不是合法版本 {latest:?}：{error}"))?;

        let active = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "版本状态锁已损坏".to_string())?;
            state.last_successful_check = Some(now);
            self.save_state(&state)?;
            state
                .active
                .clone()
                .unwrap_or_else(|| self.manifest.dsh_version.clone())
        };
        if !version_is_newer(&latest, &active) {
            return Ok(UpdateOutcome::Current);
        }

        let destination = self.paths.runtimes.join(&latest);
        if !runtime_cli(&destination).is_file() {
            self.install_and_verify(&latest, &destination)?;
        }
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "版本状态锁已损坏".to_string())?;
            state.pending = Some(latest.clone());
            self.save_state(&state)?;
        }
        Ok(UpdateOutcome::Staged(latest))
    }

    fn install_and_verify(&self, version: &str, destination: &Path) -> Result<(), String> {
        let stage = self
            .paths
            .runtimes
            .join(format!(".staging-{version}-{}", unix_time()));
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(|error| format!("无法清理更新暂存目录：{error}"))?;
        }
        fs::create_dir_all(&stage).map_err(|error| format!("无法创建更新暂存目录：{error}"))?;
        let package_json = update_package_manifest(version);
        fs::write(
            stage.join("package.json"),
            serde_json::to_vec_pretty(&package_json).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("无法写入更新清单：{error}"))?;

        append_log(
            &self.desktop_log(),
            &format!("installing {PACKAGE_NAME}@{version}"),
        );
        let mut install_command = Command::new(&self.paths.node);
        configure_background_command(&mut install_command);
        let output = install_command
            .arg(&self.paths.npm_cli)
            .args([
                "install",
                "--omit=dev",
                "--save-exact",
                "--no-audit",
                "--no-fund",
                "--package-lock=false",
            ])
            .current_dir(&stage)
            .env("npm_config_cache", &self.paths.npm_cache)
            .env("npm_config_update_notifier", "false")
            .env("npm_config_strict_allow_scripts", "true")
            .output()
            .map_err(|error| format!("无法执行 npm install：{error}"))?;
        append_log(
            &self.runtime_log(),
            &format!(
                "npm install stdout | {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ),
        );
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("npm install 失败：{message}"));
        }

        let mut version_command = Command::new(&self.paths.node);
        configure_background_command(&mut version_command);
        let version_output = version_command
            .arg(runtime_cli(&stage))
            .arg("--version")
            .output()
            .map_err(|error| format!("无法验证新版本：{error}"))?;
        let reported = String::from_utf8_lossy(&version_output.stdout)
            .trim()
            .to_string();
        if !version_output.status.success() || reported != version {
            let _ = fs::remove_dir_all(&stage);
            return Err(format!("新版本验证失败：预期 {version}，实际 {reported:?}"));
        }

        let smoke_home = self.paths.smoke.join(format!("{version}-{}", unix_time()));
        fs::create_dir_all(&smoke_home)
            .map_err(|error| format!("无法创建冒烟测试目录：{error}"))?;
        let smoke = self.launch_at(version, &stage, Some(&smoke_home), START_TIMEOUT);
        match smoke {
            Ok((mut process, _)) => process.terminate(),
            Err(error) => {
                let _ = fs::remove_dir_all(&smoke_home);
                let _ = fs::remove_dir_all(&stage);
                return Err(format!("新版本启动验证失败：{error}"));
            }
        }
        let _ = fs::remove_dir_all(&smoke_home);
        prune_other_platforms(&stage);

        if destination.exists() {
            fs::remove_dir_all(destination)
                .map_err(|error| format!("无法替换损坏的版本目录：{error}"))?;
        }
        fs::rename(&stage, destination).map_err(|error| format!("无法提交新版本：{error}"))?;
        Ok(())
    }

    fn save_state(&self, state: &RuntimeState) -> Result<(), String> {
        let temporary = self.paths.state.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("无法编码版本状态：{error}"))?;
        fs::write(&temporary, bytes).map_err(|error| format!("无法写入版本状态：{error}"))?;
        fs::rename(&temporary, &self.paths.state)
            .map_err(|error| format!("无法提交版本状态：{error}"))
    }

    fn desktop_log(&self) -> PathBuf {
        self.paths.logs.join("desktop.log")
    }

    fn runtime_log(&self) -> PathBuf {
        self.paths.logs.join("dsh.log")
    }
}

enum UpdateOutcome {
    Current,
    Staged(String),
    Skipped,
}

fn resolve_resource(root: &Path, relative: &str) -> PathBuf {
    let direct = root.join(relative);
    if direct.exists() {
        direct
    } else {
        root.join("resources").join(relative)
    }
}

fn bundled_node_resource() -> &'static str {
    if cfg!(windows) {
        "node/node.exe"
    } else {
        "node/bin/node"
    }
}

fn log_directory_opener() -> &'static str {
    if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(not(windows))]
    let _ = command;
}

fn configure_managed_command(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
}

fn runtime_cli(runtime: &Path) -> PathBuf {
    runtime.join("node_modules/@deepseek-ai/dsh/lib/bin.js")
}

fn runtime_path_env(node: &Path, runtime: &Path) -> std::ffi::OsString {
    let mut paths = vec![
        node.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        runtime.join("node_modules/.bin"),
    ];
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths)
        .ok()
        .or_else(|| env::var_os("PATH"))
        .unwrap_or_default()
}

fn user_home() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .or_else(|| {
            let drive = env::var_os("HOMEDRIVE")?;
            let path = env::var_os("HOMEPATH")?;
            Some(PathBuf::from(drive).join(path))
        })
        .or_else(|| env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read_state(path: &Path) -> Option<RuntimeState> {
    serde_json::from_reader(File::open(path).ok()?).ok()
}

fn append_log(path: &Path, message: &str) {
    let timestamp = unix_time();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn parse_ready_url(line: &str) -> Option<String> {
    let url = line.strip_prefix(READY_PREFIX)?.split_whitespace().next()?;
    if url.starts_with("http://127.0.0.1:") {
        Some(url.to_string())
    } else {
        None
    }
}

fn parse_npm_version(bytes: &[u8]) -> Result<String, String> {
    if let Ok(value) = serde_json::from_slice::<String>(bytes) {
        return Ok(value);
    }
    let value = String::from_utf8_lossy(bytes)
        .trim()
        .trim_matches('"')
        .to_string();
    if value.is_empty() {
        Err("npm view 返回了空版本".to_string())
    } else {
        Ok(value)
    }
}

fn update_package_manifest(version: &str) -> serde_json::Value {
    let dependencies = serde_json::Map::from_iter([(
        PACKAGE_NAME.to_string(),
        serde_json::Value::String(version.to_string()),
    )]);
    let allow_scripts = serde_json::Map::from_iter(
        INSTALL_SCRIPT_PACKAGES
            .into_iter()
            .map(|name| (name.to_string(), serde_json::Value::Bool(true))),
    );
    serde_json::json!({
        "private": true,
        "dependencies": dependencies,
        "allowScripts": allow_scripts,
    })
}

fn version_is_newer(candidate: &str, active: &str) -> bool {
    match (Version::parse(candidate), Version::parse(active)) {
        (Ok(candidate), Ok(active)) => candidate > active,
        _ => false,
    }
}

fn node_platform_name(rust_name: &str) -> &str {
    match rust_name {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

fn node_arch_name(rust_name: &str) -> &str {
    match rust_name {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    }
}

fn update_is_due(state: &RuntimeState, now: u64) -> bool {
    let success_due = state
        .last_successful_check
        .is_none_or(|last| now.saturating_sub(last) >= UPDATE_INTERVAL_SECS);
    let retry_due = state
        .last_update_attempt
        .is_none_or(|last| now.saturating_sub(last) >= FAILED_RETRY_SECS);
    success_due && retry_due
}

fn launch_candidates(state: &RuntimeState, bundled: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    [
        state.pending.as_deref(),
        state.active.as_deref(),
        state.previous.as_deref(),
        Some(bundled),
    ]
    .into_iter()
    .flatten()
    .filter(|version| seen.insert((*version).to_string()))
    .map(ToString::to_string)
    .collect()
}

#[cfg(unix)]
fn terminate_child(child: &mut Child, process_id: u32) {
    let process_group = process_id as i32;
    unsafe {
        libc::kill(-process_group, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
    let _ = child.wait();
}

#[cfg(windows)]
fn terminate_child(child: &mut Child, process_id: u32) {
    let mut terminate = Command::new("taskkill.exe");
    configure_background_command(&mut terminate);
    let _ = terminate
        .args(["/PID", &process_id.to_string(), "/T"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let mut force = Command::new("taskkill.exe");
    configure_background_command(&mut force);
    let _ = force
        .args(["/F", "/PID", &process_id.to_string(), "/T"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.wait();
}

fn prune_other_platforms(runtime: &Path) {
    let prebuilds = runtime.join("node_modules/node-pty/prebuilds");
    let retained = format!(
        "{}-{}",
        node_platform_name(env::consts::OS),
        node_arch_name(env::consts::ARCH)
    );
    let Ok(entries) = fs::read_dir(prebuilds) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() != retained.as_str() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_loopback_readiness_lines() {
        assert_eq!(
            parse_ready_url("dsh web: http://127.0.0.1:43121"),
            Some("http://127.0.0.1:43121".to_string())
        );
        assert_eq!(parse_ready_url("dsh web: http://0.0.0.0:43121"), None);
        assert_eq!(parse_ready_url("noise http://127.0.0.1:43121"), None);
    }

    #[test]
    fn orders_pending_before_rollback_candidates_without_duplicates() {
        let state = RuntimeState {
            active: Some("1.0.0".to_string()),
            previous: Some("0.9.0".to_string()),
            pending: Some("1.1.0".to_string()),
            ..RuntimeState::default()
        };
        assert_eq!(
            launch_candidates(&state, "1.0.0"),
            ["1.1.0", "1.0.0", "0.9.0"]
        );
    }

    #[test]
    fn respects_daily_checks_and_short_failure_retries() {
        let now = 200_000;
        assert!(update_is_due(&RuntimeState::default(), now));
        assert!(!update_is_due(
            &RuntimeState {
                last_update_attempt: Some(now - 30),
                ..RuntimeState::default()
            },
            now
        ));
        assert!(!update_is_due(
            &RuntimeState {
                last_update_attempt: Some(now - FAILED_RETRY_SECS),
                last_successful_check: Some(now - 60),
                ..RuntimeState::default()
            },
            now
        ));
        assert!(update_is_due(
            &RuntimeState {
                last_update_attempt: Some(now - FAILED_RETRY_SECS),
                last_successful_check: Some(now - UPDATE_INTERVAL_SECS),
                ..RuntimeState::default()
            },
            now
        ));
    }

    #[test]
    fn compares_prereleases_with_semver() {
        assert!(version_is_newer("0.1.0", "0.1.0-rc.6"));
        assert!(version_is_newer("0.1.0-rc.7", "0.1.0-rc.6"));
        assert!(!version_is_newer("0.1.0-rc.5", "0.1.0-rc.6"));
    }

    #[test]
    fn normalizes_rust_target_names_to_node_names() {
        assert_eq!(node_platform_name("macos"), "darwin");
        assert_eq!(node_arch_name("aarch64"), "arm64");
        assert_eq!(node_platform_name("windows"), "win32");
        assert_eq!(node_arch_name("x86_64"), "x64");
        assert_eq!(node_platform_name("linux"), "linux");
    }

    #[test]
    fn writes_the_scoped_dsh_dependency_and_install_script_policy() {
        let manifest = update_package_manifest("1.2.3");
        assert_eq!(
            manifest.pointer("/dependencies/@deepseek-ai~1dsh"),
            Some(&serde_json::Value::String("1.2.3".to_string()))
        );
        assert_eq!(
            manifest.pointer("/allowScripts/node-pty"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(manifest.pointer("/dependencies/PACKAGE_NAME").is_none());
    }
}
