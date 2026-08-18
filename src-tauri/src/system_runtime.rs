use std::{
    collections::VecDeque,
    env,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::{fs::PermissionsExt, process::CommandExt};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

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
const ERROR_TAIL_LINES: usize = 20;
const ERROR_TAIL_LINE_CHARS: usize = 1_000;
const SHELL_ENV_MARKER: &str = "__DSH_DESKTOP_ENV__";
const PATH_BLOCK_START: &str = "# >>> DSH Desktop >>>";
const PATH_BLOCK_END: &str = "# <<< DSH Desktop <<<";
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

type EnvironmentEntry = (OsString, OsString);
type UserEnvironment = Vec<EnvironmentEntry>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupInfo {
    pub url: String,
    pub dsh_version: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeState {
    last_update_attempt: Option<u64>,
    last_successful_check: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeUpdatePhase {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Installing,
    Verifying,
    Ready,
    Error,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateSnapshot {
    pub phase: RuntimeUpdatePhase,
    pub target_version: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub current_version: String,
    pub pending_version: Option<String>,
    pub node_version: String,
    pub npm_version: String,
    pub update: RuntimeUpdateSnapshot,
}

struct Paths {
    state: PathBuf,
    logs: PathBuf,
    staging: PathBuf,
    smoke: PathBuf,
}

#[derive(Clone)]
struct UserToolchain {
    environment: UserEnvironment,
    shell: Option<PathBuf>,
    node: PathBuf,
    npm: PathBuf,
    node_version: String,
    npm_version: String,
}

#[derive(Clone)]
struct RuntimeInstallation {
    toolchain: UserToolchain,
    bin_dir: PathBuf,
    dsh_command: PathBuf,
    dsh_version: String,
    update_prefix: Option<PathBuf>,
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
    state: Mutex<RuntimeState>,
    installation: Mutex<Option<RuntimeInstallation>>,
    process: Mutex<Option<ManagedProcess>>,
    startup: Mutex<Option<StartupInfo>>,
    start_gate: Mutex<()>,
    update_status: Mutex<RuntimeUpdateSnapshot>,
    update_scheduled: AtomicBool,
    update_running: AtomicBool,
    shutting_down: AtomicBool,
}

impl RuntimeManager {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let app_data_dir = node_compatible_path(&app_data_dir);
        let logs = app_data_dir.join("logs");
        let staging = app_data_dir.join("runtime-staging");
        let smoke = app_data_dir.join("runtime-smoke");
        fs::create_dir_all(&logs)?;
        fs::create_dir_all(&staging)?;
        fs::create_dir_all(&smoke)?;
        let state_path = app_data_dir.join("runtime-state.json");
        let state = read_state(&state_path).unwrap_or_default();
        append_log(
            &logs.join("desktop.log"),
            "DSH Desktop initialized; waiting for the user Node/npm environment",
        );
        Ok(Self {
            paths: Paths {
                state: state_path,
                logs,
                staging,
                smoke,
            },
            state: Mutex::new(state),
            installation: Mutex::new(None),
            process: Mutex::new(None),
            startup: Mutex::new(None),
            start_gate: Mutex::new(()),
            update_status: Mutex::new(RuntimeUpdateSnapshot::default()),
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

        let installation = self.ensure_installation()?;
        let version = installation.dsh_version.clone();
        let (process, url) = self
            .launch_at(&installation, None, START_TIMEOUT)
            .map_err(|error| format!("DSH {version} 启动失败：{error}"))?;
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
            let _ = window.set_title(&format!("DSH Desktop · DSH {version}"));
        }
        let source = installation
            .update_prefix
            .as_ref()
            .map(|prefix| format!("npm prefix {}", prefix.display()))
            .unwrap_or_else(|| "external login PATH".to_string());
        append_log(
            &self.desktop_log(),
            &format!(
                "active runtime: dsh {version} at {}, Node {}, npm {}, source {}",
                installation.dsh_command.display(),
                installation.toolchain.node_version,
                installation.toolchain.npm_version,
                source
            ),
        );
        self.set_update_status(
            RuntimeUpdatePhase::Idle,
            None,
            Some("当前版本已就绪".to_string()),
        );
        Ok(info)
    }

    pub fn schedule_update_check(self: &Arc<Self>, app: tauri::AppHandle) {
        if self.update_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(UPDATE_DELAY);
            if !manager.shutting_down.load(Ordering::Acquire) {
                manager.check_for_updates(app, false);
            }
        });
    }

    pub fn check_for_updates(self: &Arc<Self>, app: tauri::AppHandle, force: bool) -> bool {
        if self.update_running.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.set_update_status(
            RuntimeUpdatePhase::Checking,
            None,
            Some("正在查询 npm 最新版本".to_string()),
        );
        let manager = Arc::clone(self);
        thread::spawn(move || {
            let result = manager.check_for_updates_inner(&app, force);
            match result {
                Ok(UpdateOutcome::Current) => manager.set_update_status(
                    RuntimeUpdatePhase::Current,
                    None,
                    Some("已是最新版本".to_string()),
                ),
                Ok(UpdateOutcome::Available(version)) => manager.set_update_status(
                    RuntimeUpdatePhase::Available,
                    Some(version),
                    Some(manager.update_available_detail()),
                ),
                Ok(UpdateOutcome::Updated(version)) => {
                    append_log(
                        &manager.desktop_log(),
                        &format!("dsh {version} installed globally; restarting"),
                    );
                    manager.set_update_status(
                        RuntimeUpdatePhase::Ready,
                        Some(version.clone()),
                        Some("已更新，正在重启".to_string()),
                    );
                    manager.terminate();
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_title(&format!("DSH Desktop · DSH {version} 已安装"));
                    }
                    app.restart();
                }
                Ok(UpdateOutcome::Skipped) => manager.restore_resting_update_status(),
                Err(error) => {
                    append_log(
                        &manager.desktop_log(),
                        &format!("update check failed: {error}"),
                    );
                    let target_version = manager.update_snapshot().target_version;
                    manager.set_update_status(
                        RuntimeUpdatePhase::Error,
                        target_version,
                        Some(user_runtime_error(&error).to_string()),
                    );
                    if force && !manager.shutting_down.load(Ordering::Acquire) {
                        show_message(
                            &app,
                            "DSH 更新失败",
                            &format!(
                                "{}\n\n更新已经停止，详细信息已写入日志。",
                                user_runtime_error(&error)
                            ),
                            MessageDialogKind::Error,
                        );
                    }
                }
            }
            manager.update_running.store(false, Ordering::Release);
        });
        true
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        let installation = self
            .installation
            .lock()
            .ok()
            .and_then(|installation| installation.clone());
        RuntimeSnapshot {
            current_version: installation
                .as_ref()
                .map(|value| value.dsh_version.clone())
                .unwrap_or_else(|| "未初始化".to_string()),
            pending_version: None,
            node_version: installation
                .as_ref()
                .map(|value| value.toolchain.node_version.clone())
                .unwrap_or_else(|| "未检测".to_string()),
            npm_version: installation
                .as_ref()
                .map(|value| value.toolchain.npm_version.clone())
                .unwrap_or_else(|| "未检测".to_string()),
            update: self.update_snapshot(),
        }
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

    fn ensure_installation(&self) -> Result<RuntimeInstallation, String> {
        if let Some(installation) = self
            .installation
            .lock()
            .map_err(|_| "运行时状态锁已损坏".to_string())?
            .clone()
            .filter(|value| executable_file(&value.dsh_command))
        {
            return Ok(installation);
        }

        let toolchain = discover_toolchain()?;
        let desktop_prefix = user_prefix(&toolchain);
        let managed_command = global_dsh_command(&desktop_prefix);

        if let Some(dsh_command) = find_program(&toolchain.environment, "dsh") {
            let npm_prefix = if paths_equal(&dsh_command, &managed_command) {
                None
            } else {
                match npm_global_prefix(&toolchain) {
                    Ok(prefix) => Some(prefix),
                    Err(error) => {
                        append_log(
                            &self.desktop_log(),
                            &format!("could not identify npm global prefix: {error}"),
                        );
                        None
                    }
                }
            };
            let update_prefix =
                update_prefix_for_command(&dsh_command, &desktop_prefix, npm_prefix.as_deref());
            let installation =
                installation_from_command(toolchain, dsh_command.clone(), update_prefix).map_err(
                    |error| format!("检测到 DSH {}，但无法使用：{error}", dsh_command.display()),
                )?;
            append_log(
                &self.desktop_log(),
                &format!(
                    "reusing dsh {} from login PATH; update prefix {}",
                    installation.dsh_command.display(),
                    installation
                        .update_prefix
                        .as_ref()
                        .map(|prefix| prefix.display().to_string())
                        .unwrap_or_else(|| "unrecognized".to_string())
                ),
            );
            *self
                .installation
                .lock()
                .map_err(|_| "运行时状态锁已损坏".to_string())? = Some(installation.clone());
            return Ok(installation);
        }

        if executable_file(&managed_command) {
            let installation = inspect_installation(toolchain, desktop_prefix)?;
            configure_terminal_path(&installation.toolchain, &installation.bin_dir)?;
            append_log(
                &self.desktop_log(),
                &format!(
                    "reusing managed dsh {}; configured terminal PATH for {}",
                    installation.dsh_command.display(),
                    installation.bin_dir.display(),
                ),
            );
            *self
                .installation
                .lock()
                .map_err(|_| "运行时状态锁已损坏".to_string())? = Some(installation.clone());
            return Ok(installation);
        }

        let target = latest_dsh_version(&toolchain)?;
        self.set_update_status(
            RuntimeUpdatePhase::Installing,
            Some(target.clone()),
            Some(format!("正在准备 DSH {target}")),
        );
        append_log(
            &self.desktop_log(),
            &format!(
                "no usable dsh found; installing {PACKAGE_NAME}@{target} into {}",
                desktop_prefix.display()
            ),
        );
        if let Err(error) = install_global_dsh(&toolchain, &desktop_prefix, &target) {
            append_log(
                &self.desktop_log(),
                &format!("initial dsh installation failed: {error}"),
            );
            return Err(format!(
                "{} 详细信息已写入日志。",
                user_runtime_error(&error)
            ));
        }

        let installation = inspect_installation(toolchain, desktop_prefix)?;
        configure_terminal_path(&installation.toolchain, &installation.bin_dir)?;
        append_log(
            &self.desktop_log(),
            &format!(
                "installed dsh {}; configured terminal PATH for {}",
                installation.dsh_version,
                installation.bin_dir.display()
            ),
        );
        *self
            .installation
            .lock()
            .map_err(|_| "运行时状态锁已损坏".to_string())? = Some(installation.clone());
        Ok(installation)
    }

    fn launch_at(
        &self,
        installation: &RuntimeInstallation,
        dsh_home: Option<&Path>,
        timeout: Duration,
    ) -> Result<(ManagedProcess, String), String> {
        let mut command = command_for_executable(
            &installation.dsh_command,
            [OsStr::new("web"), OsStr::new("--port"), OsStr::new("0")],
        );
        apply_toolchain_environment(
            &mut command,
            &installation.toolchain,
            Some(&installation.bin_dir),
        );
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NODE_ENV", "production")
            .env("DSH_DESKTOP", "1")
            .current_dir(user_home());
        if let Some(home) = dsh_home {
            command.env("DSH_HOME", home);
        }
        configure_managed_command(&mut command);

        append_log(
            &self.desktop_log(),
            &format!("launching dsh {}", installation.dsh_version),
        );
        let mut child = command
            .spawn()
            .map_err(|error| format!("无法创建 Node 进程：{error}"))?;
        let process_id = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "无法读取 DSH 标准输出".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "无法读取 DSH 错误输出".to_string())?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let output_log = self.runtime_log();
        let error_log = output_log.clone();
        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(ERROR_TAIL_LINES)));
        let stderr_tail_writer = Arc::clone(&stderr_tail);
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                append_log(&output_log, &format!("stdout | {line}"));
                if let Some(url) = parse_ready_url(&line) {
                    let _ = ready_tx.send(url);
                }
            }
        });
        let mut stderr_thread = Some(thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                remember_stderr_line(&stderr_tail_writer, &line);
                append_log(&error_log, &format!("stderr | {line}"));
            }
        }));

        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(url) = ready_rx.try_recv() {
                return Ok((ManagedProcess { child, process_id }, url));
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if let Some(handle) = stderr_thread.take() {
                        let _ = handle.join();
                    }
                    return Err(with_stderr_tail(
                        format!("进程在就绪前退出（{status}）"),
                        &stderr_tail,
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(with_stderr_tail(
                        format!("无法读取进程状态：{error}"),
                        &stderr_tail,
                    ));
                }
            }
            if Instant::now() >= deadline {
                terminate_child(&mut child, process_id);
                if let Some(handle) = stderr_thread.take() {
                    let _ = handle.join();
                }
                return Err(with_stderr_tail(
                    format!("{timeout:?} 内没有收到就绪信号"),
                    &stderr_tail,
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn check_for_updates_inner(
        &self,
        app: &tauri::AppHandle,
        force: bool,
    ) -> Result<UpdateOutcome, String> {
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

        let installation = self.ensure_installation()?;
        let latest = latest_dsh_version(&installation.toolchain)?;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "版本状态锁已损坏".to_string())?;
            state.last_successful_check = Some(now);
            self.save_state(&state)?;
        }
        if !version_is_newer(&latest, &installation.dsh_version) {
            return Ok(UpdateOutcome::Current);
        }
        if !force {
            return Ok(UpdateOutcome::Available(latest));
        }

        self.set_update_status(
            RuntimeUpdatePhase::Available,
            Some(latest.clone()),
            Some(self.update_available_detail()),
        );
        let Some(update_prefix) = installation.update_prefix.as_ref() else {
            show_message(
                app,
                "请使用原安装方式更新 DSH",
                &format!(
                    "发现 DSH {latest}，但无法确认当前 DSH {} 的安装方式。\n\n为避免改变安装位置，DSH Desktop 不会自动更新它。请使用原来的包管理器完成更新。",
                    installation.dsh_command.display()
                ),
                MessageDialogKind::Info,
            );
            return Ok(UpdateOutcome::Available(latest));
        };
        let accepted = app
            .dialog()
            .message(format!(
                "发现 DSH {latest}。\n\n更新会在原安装位置 {} 替换 DSH，不会改变命令位置。\n\n安装前会先在临时目录完成启动验证。",
                update_prefix.display()
            ))
            .title("DSH 更新")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "更新并重新启动".to_string(),
                "稍后".to_string(),
            ))
            .blocking_show();
        if !accepted {
            return Ok(UpdateOutcome::Available(latest));
        }

        self.set_update_status(
            RuntimeUpdatePhase::Installing,
            Some(latest.clone()),
            Some(format!("正在准备 DSH {latest}")),
        );
        let updated = self.install_and_verify_update(&installation, &latest)?;
        *self
            .installation
            .lock()
            .map_err(|_| "运行时状态锁已损坏".to_string())? = Some(updated);
        Ok(UpdateOutcome::Updated(latest))
    }

    fn install_and_verify_update(
        &self,
        current: &RuntimeInstallation,
        version: &str,
    ) -> Result<RuntimeInstallation, String> {
        let update_prefix = current
            .update_prefix
            .as_ref()
            .ok_or_else(|| "当前 DSH 的安装方式无法识别，不能自动更新".to_string())?;
        let stage = self
            .paths
            .staging
            .join(format!("{version}-{}", unix_time()));
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(|error| format!("无法清理更新暂存目录：{error}"))?;
        }
        fs::create_dir_all(&stage).map_err(|error| format!("无法创建更新暂存目录：{error}"))?;
        append_log(
            &self.desktop_log(),
            &format!("staging {PACKAGE_NAME}@{version}"),
        );
        if let Err(error) = install_global_dsh(&current.toolchain, &stage, version) {
            let _ = fs::remove_dir_all(&stage);
            return Err(error);
        }
        let staged = match inspect_installation(current.toolchain.clone(), stage.clone()) {
            Ok(value) => value,
            Err(error) => {
                let _ = fs::remove_dir_all(&stage);
                return Err(format!("新版本验证失败：{error}"));
            }
        };
        if staged.dsh_version != version {
            let _ = fs::remove_dir_all(&stage);
            return Err(format!(
                "新版本验证失败：预期 {version}，实际 {}",
                staged.dsh_version
            ));
        }

        self.set_update_status(
            RuntimeUpdatePhase::Verifying,
            Some(version.to_string()),
            Some("正在验证版本与启动能力".to_string()),
        );
        let smoke_home = self.paths.smoke.join(format!("{version}-{}", unix_time()));
        fs::create_dir_all(&smoke_home)
            .map_err(|error| format!("无法创建冒烟测试目录：{error}"))?;
        match self.launch_at(&staged, Some(&smoke_home), START_TIMEOUT) {
            Ok((mut process, _)) => process.terminate(),
            Err(error) => {
                let _ = fs::remove_dir_all(&smoke_home);
                let _ = fs::remove_dir_all(&stage);
                return Err(format!("新版本启动验证失败：{error}"));
            }
        }
        let _ = fs::remove_dir_all(&smoke_home);
        let _ = fs::remove_dir_all(&stage);

        self.set_update_status(
            RuntimeUpdatePhase::Installing,
            Some(version.to_string()),
            Some(format!("正在原安装位置更新 DSH {version}")),
        );
        if let Err(error) = install_global_dsh(&current.toolchain, update_prefix, version) {
            return Err(self.rollback_after_failed_update(current, &error));
        }
        match inspect_installation(current.toolchain.clone(), update_prefix.clone()) {
            Ok(installed) if installed.dsh_version == version => Ok(installed),
            Ok(installed) => Err(self.rollback_after_failed_update(
                current,
                &format!(
                    "安装后版本不匹配：预期 {version}，实际 {}",
                    installed.dsh_version
                ),
            )),
            Err(error) => Err(self.rollback_after_failed_update(current, &error)),
        }
    }

    fn rollback_after_failed_update(
        &self,
        current: &RuntimeInstallation,
        update_error: &str,
    ) -> String {
        let Some(update_prefix) = current.update_prefix.as_ref() else {
            append_log(
                &self.desktop_log(),
                &format!(
                    "refused to modify unrecognized dsh {} after update error: {update_error}",
                    current.dsh_command.display()
                ),
            );
            return format!("{update_error}；现有 DSH {} 未被修改", current.dsh_version);
        };
        append_log(
            &self.desktop_log(),
            &format!(
                "global update failed; reinstalling dsh {}: {update_error}",
                current.dsh_version
            ),
        );
        match install_global_dsh(&current.toolchain, update_prefix, &current.dsh_version) {
            Ok(()) => format!("{update_error}；已恢复 DSH {}", current.dsh_version),
            Err(rollback_error) => format!(
                "{update_error}；恢复 DSH {} 也失败：{rollback_error}",
                current.dsh_version
            ),
        }
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

    fn update_snapshot(&self) -> RuntimeUpdateSnapshot {
        self.update_status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| RuntimeUpdateSnapshot {
                phase: RuntimeUpdatePhase::Error,
                detail: Some("无法读取 DSH 更新状态".to_string()),
                ..RuntimeUpdateSnapshot::default()
            })
    }

    fn set_update_status(
        &self,
        phase: RuntimeUpdatePhase,
        target_version: Option<String>,
        detail: Option<String>,
    ) {
        if let Ok(mut status) = self.update_status.lock() {
            *status = RuntimeUpdateSnapshot {
                phase,
                target_version,
                detail,
            };
        }
    }

    fn restore_resting_update_status(&self) {
        let current = self.installation.lock().ok().and_then(|value| {
            value
                .as_ref()
                .map(|installation| installation.dsh_version.clone())
        });
        self.set_update_status(
            RuntimeUpdatePhase::Idle,
            None,
            current.map(|version| format!("当前版本 {version}")),
        );
    }

    fn update_available_detail(&self) -> String {
        self.installation
            .lock()
            .ok()
            .and_then(|installation| {
                installation.as_ref().map(|installation| {
                    if installation.update_prefix.is_some() {
                        "将在当前 DSH 的原安装位置更新".to_string()
                    } else {
                        "请使用原来的安装方式更新 DSH".to_string()
                    }
                })
            })
            .unwrap_or_else(|| "发现 DSH 新版本".to_string())
    }
}

enum UpdateOutcome {
    Current,
    Available(String),
    Updated(String),
    Skipped,
}

fn discover_toolchain() -> Result<UserToolchain, String> {
    let (environment, shell) = user_environment()?;
    let node = find_program(&environment, "node").ok_or_else(|| {
        "未检测到 Node.js。请先安装 Node.js 24 LTS，然后重新打开 DSH Desktop。".to_string()
    })?;
    let npm = find_program(&environment, "npm").ok_or_else(|| {
        "检测到 Node.js，但没有找到 npm。请安装包含 npm 的 Node.js 24 LTS。".to_string()
    })?;
    let provisional = UserToolchain {
        environment,
        shell,
        node,
        npm,
        node_version: String::new(),
        npm_version: String::new(),
    };
    let node_version = command_text(
        command_for_tool(
            &provisional,
            &provisional.node,
            [OsStr::new("--version")],
            None,
        ),
        "node --version",
    )?
    .trim_start_matches('v')
    .to_string();
    if !supported_node_version(&node_version) {
        return Err(format!(
            "Node.js {node_version} 不受支持。需要 Node.js 22.19+（22 系列）或 24 及以上版本。"
        ));
    }
    let npm_version = command_text(
        command_for_tool(
            &provisional,
            &provisional.npm,
            [OsStr::new("--version")],
            None,
        ),
        "npm --version",
    )?;
    Ok(UserToolchain {
        node_version,
        npm_version,
        ..provisional
    })
}

fn inspect_installation(
    toolchain: UserToolchain,
    prefix: PathBuf,
) -> Result<RuntimeInstallation, String> {
    let dsh_command = global_dsh_command(&prefix);
    installation_from_command(toolchain, dsh_command, Some(prefix))
}

fn installation_from_command(
    toolchain: UserToolchain,
    dsh_command: PathBuf,
    update_prefix: Option<PathBuf>,
) -> Result<RuntimeInstallation, String> {
    let bin_dir = dsh_command
        .parent()
        .ok_or_else(|| format!("DSH 命令没有父目录：{}", dsh_command.display()))?
        .to_path_buf();
    let dsh_version = read_dsh_version(&toolchain, &dsh_command, &bin_dir)?;
    Ok(RuntimeInstallation {
        toolchain,
        bin_dir,
        dsh_command,
        dsh_version,
        update_prefix,
    })
}

fn npm_global_prefix(toolchain: &UserToolchain) -> Result<PathBuf, String> {
    let output = command_text(
        command_for_tool(
            toolchain,
            &toolchain.npm,
            [OsStr::new("prefix"), OsStr::new("--global")],
            None,
        ),
        "npm prefix --global",
    )?;
    let prefix = node_compatible_path(Path::new(&output));
    if !prefix.is_absolute() {
        return Err(format!("npm 返回了非绝对全局目录：{output}"));
    }
    Ok(prefix)
}

fn update_prefix_for_command(
    dsh_command: &Path,
    desktop_prefix: &Path,
    npm_prefix: Option<&Path>,
) -> Option<PathBuf> {
    if paths_equal(dsh_command, &global_dsh_command(desktop_prefix)) {
        return Some(desktop_prefix.to_path_buf());
    }
    npm_prefix
        .filter(|prefix| paths_equal(dsh_command, &global_dsh_command(prefix)))
        .map(Path::to_path_buf)
}

fn read_dsh_version(
    toolchain: &UserToolchain,
    dsh_command: &Path,
    bin_dir: &Path,
) -> Result<String, String> {
    if !executable_file(dsh_command) {
        return Err(format!("DSH 命令不可执行：{}", dsh_command.display()));
    }
    let output = command_text(
        command_for_tool(
            toolchain,
            dsh_command,
            [OsStr::new("--version")],
            Some(bin_dir),
        ),
        "dsh --version",
    )?;
    Version::parse(&output).map_err(|error| format!("DSH 返回了无效版本 {output:?}：{error}"))?;
    Ok(output)
}

fn latest_dsh_version(toolchain: &UserToolchain) -> Result<String, String> {
    let output = command_output(
        command_for_tool(
            toolchain,
            &toolchain.npm,
            [
                OsStr::new("view"),
                OsStr::new(PACKAGE_NAME),
                OsStr::new("dist-tags.latest"),
                OsStr::new("--json"),
            ],
            None,
        ),
        "npm view",
    )?;
    let latest = parse_npm_version(&output.stdout)?;
    Version::parse(&latest)
        .map_err(|error| format!("npm latest 不是合法版本 {latest:?}：{error}"))?;
    Ok(latest)
}

fn dsh_publish_time(toolchain: &UserToolchain, dsh_version: &str) -> Result<String, String> {
    let package = format!("{PACKAGE_NAME}@{dsh_version}");
    let output = command_output(
        command_for_tool(
            toolchain,
            &toolchain.npm,
            [
                OsStr::new("view"),
                OsStr::new(&package),
                OsStr::new("time"),
                OsStr::new("--json"),
            ],
            None,
        ),
        "npm view DSH publish time",
    )?;
    parse_npm_publish_time(&output.stdout, dsh_version)
}

fn install_global_dsh(
    toolchain: &UserToolchain,
    prefix: &Path,
    dsh_version: &str,
) -> Result<(), String> {
    Version::parse(dsh_version)
        .map_err(|error| format!("拒绝安装无效的 DSH 版本 {dsh_version:?}：{error}"))?;
    let publish_time = dsh_publish_time(toolchain, dsh_version)?;
    for directory in [
        prefix.to_path_buf(),
        global_modules_dir(prefix),
        global_bin_dir(prefix),
    ] {
        fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建 {}：{error}", directory.display()))?;
    }
    let dsh_spec = format!("{PACKAGE_NAME}@{dsh_version}");
    let install_arguments = vec![
        OsString::from("install"),
        OsString::from("--global"),
        OsString::from("--prefix"),
        prefix.as_os_str().to_os_string(),
        OsString::from("--omit=dev"),
        OsString::from("--no-audit"),
        OsString::from("--no-fund"),
        OsString::from("--package-lock=false"),
        OsString::from("--ignore-scripts"),
        OsString::from(format!("--before={publish_time}")),
        OsString::from(dsh_spec),
    ];
    let mut command = command_for_tool(toolchain, &toolchain.npm, install_arguments, None);
    command
        .env("npm_config_update_notifier", "false")
        .env("npm_config_audit", "false")
        .env("npm_config_fund", "false");
    command_output(command, "npm install --global")?;

    let mut rebuild_arguments = vec![
        OsString::from("rebuild"),
        OsString::from("--global"),
        OsString::from("--prefix"),
        prefix.as_os_str().to_os_string(),
        OsString::from("--ignore-scripts=false"),
        OsString::from("--no-audit"),
        OsString::from("--no-fund"),
    ];
    if npm_supports_allow_scripts(&toolchain.npm_version) {
        rebuild_arguments.extend([
            OsString::from(format!(
                "--allow-scripts={}",
                INSTALL_SCRIPT_PACKAGES.join(",")
            )),
            OsString::from("--strict-allow-scripts"),
        ]);
    }
    rebuild_arguments.extend(INSTALL_SCRIPT_PACKAGES.map(OsString::from));
    let mut rebuild = command_for_tool(toolchain, &toolchain.npm, rebuild_arguments, None);
    rebuild
        .env("npm_config_update_notifier", "false")
        .env("npm_config_audit", "false")
        .env("npm_config_fund", "false");
    command_output(rebuild, "npm rebuild DSH native dependencies")?;
    Ok(())
}

fn user_prefix(toolchain: &UserToolchain) -> PathBuf {
    #[cfg(windows)]
    {
        environment_value(&toolchain.environment, "APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home().join("AppData/Roaming"))
            .join("npm")
    }
    #[cfg(not(windows))]
    {
        let _ = toolchain;
        user_home().join(".local")
    }
}

fn global_modules_dir(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.join("node_modules")
    } else {
        prefix.join("lib/node_modules")
    }
}

fn global_bin_dir(prefix: &Path) -> PathBuf {
    if cfg!(windows) {
        prefix.to_path_buf()
    } else {
        prefix.join("bin")
    }
}

fn global_dsh_command(prefix: &Path) -> PathBuf {
    let bin = global_bin_dir(prefix);
    if cfg!(windows) {
        bin.join("dsh.cmd")
    } else {
        bin.join("dsh")
    }
}

fn configure_terminal_path(toolchain: &UserToolchain, bin_dir: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = toolchain;
        let script = r#"
$target = $env:DSH_DESKTOP_BIN
if ([string]::IsNullOrWhiteSpace($target)) { throw 'DSH_DESKTOP_BIN is empty' }
$current = [Environment]::GetEnvironmentVariable('Path', [EnvironmentVariableTarget]::User)
$entries = @($current -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
$expandedTarget = [Environment]::ExpandEnvironmentVariables($target).TrimEnd('\')
$present = @($entries | Where-Object {
  [string]::Equals(
    [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\'),
    $expandedTarget,
    [StringComparison]::OrdinalIgnoreCase
  )
}).Count -gt 0
if (-not $present) {
  $updated = if ([string]::IsNullOrWhiteSpace($current)) { $target } else { "$target;$current" }
  [Environment]::SetEnvironmentVariable('Path', $updated, [EnvironmentVariableTarget]::User)
}
"#;
        let mut command = Command::new("powershell.exe");
        configure_background_command(&mut command);
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .env("DSH_DESKTOP_BIN", bin_dir);
        command_output(command, "更新当前用户 PATH")?;
        prepend_process_path(bin_dir);
        broadcast_environment_change();
        Ok(())
    }
    #[cfg(unix)]
    {
        let shell = toolchain
            .shell
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(OsStr::to_str)
            .unwrap_or("zsh");
        let (profile, path_line) = match shell {
            "zsh" => (
                user_home().join(".zprofile"),
                "export PATH=\"$HOME/.local/bin:$PATH\"",
            ),
            "bash" => (
                user_home().join(".bash_profile"),
                "export PATH=\"$HOME/.local/bin:$PATH\"",
            ),
            "fish" => (
                user_home().join(".config/fish/config.fish"),
                "fish_add_path \"$HOME/.local/bin\"",
            ),
            other => {
                return Err(format!(
                    "DSH 已安装，但暂不支持自动配置 {other}。请手动把 {} 加入 PATH。",
                    bin_dir.display()
                ));
            }
        };
        if let Some(parent) = profile.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("无法创建终端配置目录：{error}"))?;
        }
        let existing = fs::read_to_string(&profile).unwrap_or_default();
        if existing.contains(PATH_BLOCK_START) || existing.contains("$HOME/.local/bin") {
            return Ok(());
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&profile)
            .map_err(|error| format!("无法打开 {}：{error}", profile.display()))?;
        if !existing.is_empty() && !existing.ends_with('\n') {
            writeln!(file).map_err(|error| format!("无法更新 {}：{error}", profile.display()))?;
        }
        writeln!(file, "{PATH_BLOCK_START}\n{path_line}\n{PATH_BLOCK_END}")
            .map_err(|error| format!("无法更新 {}：{error}", profile.display()))?;
        Ok(())
    }
}

fn user_environment() -> Result<(UserEnvironment, Option<PathBuf>), String> {
    #[cfg(windows)]
    {
        Ok((env::vars_os().collect(), None))
    }
    #[cfg(unix)]
    {
        let shell = env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .unwrap_or_else(|| PathBuf::from("/bin/zsh"));
        let output = Command::new(&shell)
            .args(["-lic", &format!("printf '{SHELL_ENV_MARKER}\\n'; env")])
            .output()
            .map_err(|error| format!("无法读取登录终端环境：{error}"))?;
        let text = String::from_utf8_lossy(&output.stdout);
        let marker = format!("{SHELL_ENV_MARKER}\n");
        let environment_text = text
            .rsplit_once(&marker)
            .map(|(_, value)| value)
            .ok_or_else(|| {
                let detail = String::from_utf8_lossy(&output.stderr);
                format!("登录终端没有返回环境变量：{}", detail.trim())
            })?;
        let mut environment = Vec::new();
        for line in environment_text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !key.is_empty() {
                environment.push((OsString::from(key), OsString::from(value)));
            }
        }
        if environment_value(&environment, "PATH").is_none() {
            return Err("登录终端没有提供 PATH。请确认 Node.js 可以在新终端中运行。".to_string());
        }
        Ok((environment, Some(shell)))
    }
}

fn find_program(environment: &[EnvironmentEntry], name: &str) -> Option<PathBuf> {
    let path = environment_value(environment, "PATH")?;
    for directory in env::split_paths(&path) {
        #[cfg(windows)]
        let candidates = [
            directory.join(format!("{name}.exe")),
            directory.join(format!("{name}.cmd")),
            directory.join(format!("{name}.bat")),
            directory.join(name),
        ];
        #[cfg(not(windows))]
        let candidates = [directory.join(name)];
        for candidate in candidates {
            if executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        true
    }
}

fn command_for_tool<I, S>(
    toolchain: &UserToolchain,
    executable: &Path,
    arguments: I,
    prefix_bin: Option<&Path>,
) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = command_for_executable(executable, arguments);
    apply_toolchain_environment(&mut command, toolchain, prefix_bin);
    configure_background_command(&mut command);
    command
}

fn command_for_executable<I, S>(executable: &Path, arguments: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    #[cfg(windows)]
    let mut command = if executable
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat"))
    {
        let mut command =
            Command::new(env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")));
        command.args(["/d", "/s", "/c"]).arg(executable);
        command
    } else {
        Command::new(executable)
    };
    #[cfg(not(windows))]
    let mut command = Command::new(executable);
    command.args(arguments);
    command
}

fn apply_toolchain_environment(
    command: &mut Command,
    toolchain: &UserToolchain,
    prefix_bin: Option<&Path>,
) {
    command
        .env_clear()
        .envs(toolchain.environment.iter().cloned());
    if let Some(prefix_bin) = prefix_bin {
        let mut paths = vec![prefix_bin.to_path_buf()];
        if let Some(existing) = environment_value(&toolchain.environment, "PATH") {
            paths.extend(env::split_paths(&existing));
        }
        if let Ok(path) = env::join_paths(paths) {
            command.env("PATH", path);
        }
    }
}

fn command_output(mut command: Command, description: &str) -> Result<Output, String> {
    let output = command
        .output()
        .map_err(|error| format!("无法执行 {description}：{error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        Err(format!("{description} 失败：{detail}"))
    }
}

fn command_text(command: Command, description: &str) -> Result<String, String> {
    let output = command_output(command, description)?;
    let text = String::from_utf8(output.stdout)
        .map_err(|error| format!("{description} 返回了无效文本：{error}"))?
        .trim()
        .to_string();
    if text.is_empty() {
        Err(format!("{description} 没有返回版本"))
    } else {
        Ok(text)
    }
}

fn environment_value(environment: &[EnvironmentEntry], name: &str) -> Option<OsString> {
    environment.iter().find_map(|(key, value)| {
        let matches = if cfg!(windows) {
            key.to_string_lossy().eq_ignore_ascii_case(name)
        } else {
            key == OsStr::new(name)
        };
        matches.then(|| value.clone())
    })
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn supported_node_version(value: &str) -> bool {
    Version::parse(value).is_ok_and(|version| {
        (version.major == 22 && version >= Version::new(22, 19, 0)) || version.major >= 24
    })
}

fn npm_supports_allow_scripts(value: &str) -> bool {
    Version::parse(value).is_ok_and(|version| version >= Version::new(11, 16, 0))
}

#[cfg(windows)]
fn prepend_process_path(bin_dir: &Path) {
    let mut paths = vec![bin_dir.to_path_buf()];
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    if let Ok(path) = env::join_paths(paths) {
        env::set_var("PATH", path);
    }
}

#[cfg(windows)]
fn broadcast_environment_change() {
    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            window: isize,
            message: u32,
            word: usize,
            data: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001a;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    let environment = "Environment\0".encode_utf16().collect::<Vec<_>>();
    let mut result = 0;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5_000,
            &mut result,
        );
    }
}

fn node_compatible_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        dunce::simplified(path).to_path_buf()
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
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

fn remember_stderr_line(tail: &Mutex<VecDeque<String>>, line: &str) {
    let mut captured = line.chars().take(ERROR_TAIL_LINE_CHARS).collect::<String>();
    if line.chars().count() > ERROR_TAIL_LINE_CHARS {
        captured.push('…');
    }
    if let Ok(mut lines) = tail.lock() {
        if lines.len() == ERROR_TAIL_LINES {
            lines.pop_front();
        }
        lines.push_back(captured);
    }
}

fn with_stderr_tail(message: String, tail: &Mutex<VecDeque<String>>) -> String {
    let Ok(lines) = tail.lock() else {
        return message;
    };
    if lines.is_empty() {
        message
    } else {
        format!(
            "{message}\n\n关键日志：\n{}",
            lines.iter().cloned().collect::<Vec<_>>().join("\n")
        )
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

fn parse_npm_publish_time(bytes: &[u8], version: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("npm view 返回了无效的发布时间：{error}"))?;
    let published = value
        .as_object()
        .and_then(|times| times.get(version))
        .and_then(serde_json::Value::as_str)
        .filter(|time| time.contains('T') && time.ends_with('Z'))
        .ok_or_else(|| format!("npm view 没有返回 DSH {version} 的发布时间"))?;
    Ok(published.to_string())
}

fn version_is_newer(candidate: &str, active: &str) -> bool {
    match (Version::parse(candidate), Version::parse(active)) {
        (Ok(candidate), Ok(active)) => candidate > active,
        _ => false,
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

fn user_runtime_error(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("node.js") || lower.contains("node --version") {
        "没有找到兼容的 Node.js，请先安装 Node.js 24 LTS。"
    } else if lower.contains("e404") || lower.contains("404 not found") {
        "npm 镜像中的包文件暂未同步，请稍后重试。"
    } else if [
        "network",
        "connect",
        "fetch",
        "timed out",
        "timeout",
        "dns",
        "econn",
        "enet",
        "eai_again",
        "socket",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        "无法连接 npm，请检查网络或 npm 代理配置后重试。"
    } else if lower.contains("permission") || lower.contains("eacces") {
        if cfg!(windows) {
            "无法写入当前用户的 npm 目录，请检查目录权限。"
        } else {
            "无法写入用户级安装目录，请检查 ~/.local 的权限。"
        }
    } else {
        "暂时无法完成 DSH 更新，请稍后重试。"
    }
}

fn show_message(app: &tauri::AppHandle, title: &str, message: &str, kind: MessageDialogKind) {
    app.dialog()
        .message(message)
        .title(title)
        .kind(kind)
        .buttons(MessageDialogButtons::Ok)
        .blocking_show();
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
    fn accepts_only_supported_node_release_lines() {
        assert!(!supported_node_version("22.18.0"));
        assert!(supported_node_version("22.19.0"));
        assert!(!supported_node_version("23.11.1"));
        assert!(supported_node_version("24.0.0"));
        assert!(supported_node_version("26.1.0"));
    }

    #[test]
    fn enables_install_script_allowlists_only_when_npm_supports_them() {
        assert!(!npm_supports_allow_scripts("10.9.4"));
        assert!(!npm_supports_allow_scripts("11.15.1"));
        assert!(npm_supports_allow_scripts("11.16.0"));
        assert!(npm_supports_allow_scripts("12.0.0"));
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
        assert!(update_is_due(
            &RuntimeState {
                last_update_attempt: Some(now - FAILED_RETRY_SECS),
                last_successful_check: Some(now - UPDATE_INTERVAL_SECS),
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
    fn keeps_recent_stderr_in_the_startup_error() {
        let tail = Mutex::new(VecDeque::new());
        remember_stderr_line(&tail, "first failure");
        remember_stderr_line(&tail, "second failure");
        assert_eq!(
            with_stderr_tail("启动失败".to_string(), &tail),
            "启动失败\n\n关键日志：\nfirst failure\nsecond failure"
        );
    }

    #[test]
    fn resolves_user_global_layout() {
        let prefix = Path::new("/tmp/dsh-user-prefix");
        if cfg!(windows) {
            assert_eq!(global_modules_dir(prefix), prefix.join("node_modules"));
            assert_eq!(global_bin_dir(prefix), prefix);
            assert_eq!(global_dsh_command(prefix), prefix.join("dsh.cmd"));
        } else {
            assert_eq!(global_modules_dir(prefix), prefix.join("lib/node_modules"));
            assert_eq!(global_bin_dir(prefix), prefix.join("bin"));
            assert_eq!(global_dsh_command(prefix), prefix.join("bin/dsh"));
        }
    }

    #[test]
    fn preserves_the_existing_dsh_install_prefix() {
        let desktop_prefix = Path::new("/tmp/dsh-desktop-prefix");
        let npm_prefix = Path::new("/tmp/user-node-prefix");
        assert_eq!(
            update_prefix_for_command(
                &global_dsh_command(desktop_prefix),
                desktop_prefix,
                Some(npm_prefix),
            ),
            Some(desktop_prefix.to_path_buf())
        );
        assert_eq!(
            update_prefix_for_command(
                &global_dsh_command(npm_prefix),
                desktop_prefix,
                Some(npm_prefix),
            ),
            Some(npm_prefix.to_path_buf())
        );
        assert_eq!(
            update_prefix_for_command(
                Path::new("/opt/unknown/bin/dsh"),
                desktop_prefix,
                Some(npm_prefix),
            ),
            None
        );
    }

    #[test]
    fn selects_the_exact_dsh_publish_time() {
        let time = parse_npm_publish_time(
            br#"{"created":"2026-08-01T00:00:00.000Z","0.1.0-rc.7":"2026-08-17T11:50:59.194Z"}"#,
            "0.1.0-rc.7",
        );
        assert_eq!(time.as_deref(), Ok("2026-08-17T11:50:59.194Z"));
        assert!(parse_npm_publish_time(br#"{"0.1.0":"yesterday"}"#, "0.1.0").is_err());
        assert!(
            parse_npm_publish_time(br#"{"0.1.0":"2026-08-17T11:50:59.194Z"}"#, "0.2.0").is_err()
        );
    }

    #[test]
    fn gives_a_specific_message_for_registry_propagation_failures() {
        assert_eq!(
            user_runtime_error("npm error code E404\n404 Not Found"),
            "npm 镜像中的包文件暂未同步，请稍后重试。"
        );
    }
}
