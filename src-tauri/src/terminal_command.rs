use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SHIM_MARKER: &str = "Managed by DSH Desktop";
const PATH_BLOCK_START: &str = "# >>> DSH Desktop terminal command >>>";
const PATH_BLOCK_END: &str = "# <<< DSH Desktop terminal command <<<";
const LAUNCHER_NAME: &str = "terminal-launcher.mjs";
const CONFIG_NAME: &str = "terminal-runtime.json";
const METADATA_NAME: &str = "terminal-command.json";

const TERMINAL_LAUNCHER: &str = r#"import { existsSync, readFileSync } from "node:fs";
import { dirname, delimiter, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const config = JSON.parse(readFileSync(join(root, "terminal-runtime.json"), "utf8"));
const requestedCwd = process.env.DSH_DESKTOP_TERMINAL_CWD;
if (requestedCwd) process.chdir(requestedCwd);

let state = {};
try {
  state = JSON.parse(readFileSync(config.statePath, "utf8"));
} catch {}

const installed = state.active ? join(config.runtimesPath, state.active) : undefined;
const candidates = [installed, config.bundledRuntime].filter(Boolean);
const runtime = candidates.find((candidate) =>
  existsSync(join(candidate, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js")),
);
if (!runtime) {
  console.error("DSH Desktop 找不到可用的 DSH 运行时，请先打开桌面应用。");
  process.exit(1);
}

const cli = join(runtime, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js");
const runtimeBin = join(runtime, "node_modules", ".bin");
process.env.PATH = [dirname(process.execPath), runtimeBin, process.env.PATH]
  .filter(Boolean)
  .join(delimiter);
process.argv[1] = cli;
await import(pathToFileURL(cli).href);
"#;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LauncherConfig {
    state_path: PathBuf,
    runtimes_path: PathBuf,
    bundled_runtime: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallMetadata {
    command: String,
    shim_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    shell_config_paths: Vec<PathBuf>,
    path_added: bool,
}

#[derive(Clone, Debug)]
pub struct TerminalCommandInstall {
    pub command: String,
    pub shim_path: PathBuf,
    pub configured_shells: Vec<String>,
}

pub struct TerminalCommandManager {
    root: PathBuf,
    node: PathBuf,
    bundled_runtime: PathBuf,
    runtimes: PathBuf,
    state: PathBuf,
}

impl TerminalCommandManager {
    pub fn new(
        app_data_dir: &Path,
        node: PathBuf,
        bundled_runtime: PathBuf,
        runtimes: PathBuf,
        state: PathBuf,
    ) -> Self {
        Self {
            root: app_data_dir.join("terminal"),
            node,
            bundled_runtime,
            runtimes,
            state,
        }
    }

    pub fn refresh_if_installed(&self) {
        let Some(metadata) = self.read_metadata() else {
            return;
        };
        if !is_managed_shim(&metadata.shim_path) {
            return;
        }
        if self.write_launcher_assets().is_ok() {
            let _ = self.write_shim(&metadata.shim_path);
        }
    }

    pub fn install(&self) -> Result<TerminalCommandInstall, String> {
        fs::create_dir_all(&self.root).map_err(|error| format!("无法创建终端命令目录：{error}"))?;
        self.write_launcher_assets()?;

        let bin_dir = self.bin_dir();
        fs::create_dir_all(&bin_dir)
            .map_err(|error| format!("无法创建 {}：{error}", bin_dir.display()))?;

        let existing_metadata = self.read_metadata();
        let dsh_target = shim_path(&bin_dir, "dsh");
        let dsh_target_is_managed = is_managed_shim(&dsh_target);
        let existing_dsh = resolve_command("dsh");
        let managed_command = existing_metadata
            .as_ref()
            .filter(|metadata| is_managed_shim(&metadata.shim_path))
            .and_then(|metadata| match metadata.command.as_str() {
                "dsh"
                    if !existing_dsh.exists
                        || existing_dsh
                            .path
                            .as_deref()
                            .is_some_and(|path| paths_equal(path, &metadata.shim_path)) =>
                {
                    Some("dsh")
                }
                "dsh-desktop" => Some("dsh-desktop"),
                _ => None,
            })
            .or_else(|| {
                (dsh_target_is_managed
                    && (!existing_dsh.exists
                        || existing_dsh
                            .path
                            .as_deref()
                            .is_some_and(|path| paths_equal(path, &dsh_target))))
                .then_some("dsh")
            });
        let command = choose_command_name(
            managed_command,
            existing_dsh.exists,
            dsh_target.exists() && !dsh_target_is_managed,
        );
        let target = shim_path(&bin_dir, command);
        if command == "dsh-desktop" {
            let existing_desktop = resolve_command("dsh-desktop");
            if existing_desktop.exists
                && !existing_desktop
                    .path
                    .as_deref()
                    .is_some_and(|path| paths_equal(path, &target))
            {
                return Err(
                    "终端中已经有另一个 dsh-desktop，DSH Desktop 不会改变它的优先级。".to_string(),
                );
            }
        }
        if command == "dsh-desktop" && dsh_target.exists() {
            if !dsh_target_is_managed {
                return Err(format!(
                    "{} 已存在；加入命令目录会改变它的优先级，因此 DSH Desktop 没有修改 PATH。",
                    dsh_target.display()
                ));
            }
            fs::remove_file(&dsh_target)
                .map_err(|error| format!("无法移除旧的 DSH Desktop 命令：{error}"))?;
        }
        if target.exists() && !is_managed_shim(&target) {
            return Err(format!(
                "{} 已存在，DSH Desktop 不会覆盖它。请先确认该文件的用途。",
                target.display()
            ));
        }

        if let Some(previous) = existing_metadata.as_ref() {
            if previous.shim_path != target && is_managed_shim(&previous.shim_path) {
                fs::remove_file(&previous.shim_path)
                    .map_err(|error| format!("无法移除旧的 DSH Desktop 命令：{error}"))?;
            }
        }

        self.write_shim(&target)?;
        let legacy_profile = existing_metadata
            .as_ref()
            .filter(|metadata| metadata.path_added)
            .and_then(|metadata| metadata.profile_path.as_deref());
        let path_change = configure_terminal_path(
            &bin_dir,
            legacy_profile,
            existing_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.path_added),
        )?;
        let PathChange {
            config_paths,
            configured_shells,
            added,
        } = path_change;
        let metadata = InstallMetadata {
            command: command.to_string(),
            shim_path: target.clone(),
            profile_path: None,
            shell_config_paths: config_paths,
            path_added: added,
        };
        fs::write(
            self.root.join(METADATA_NAME),
            serde_json::to_vec_pretty(&metadata).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("无法保存终端命令状态：{error}"))?;

        Ok(TerminalCommandInstall {
            command: metadata.command,
            shim_path: target,
            configured_shells,
        })
    }

    pub fn uninstall(&self) -> Result<String, String> {
        let metadata = self
            .read_metadata()
            .ok_or_else(|| "没有由 DSH Desktop 安装的终端命令。".to_string())?;
        let mut config_paths = metadata.shell_config_paths.clone();
        if let Some(profile_path) = metadata.profile_path.as_ref() {
            config_paths.push(profile_path.clone());
        }
        if metadata.shim_path.exists() && !is_managed_shim(&metadata.shim_path) {
            return Err(format!(
                "{} 已被修改，DSH Desktop 不会删除它。",
                metadata.shim_path.display()
            ));
        }
        if metadata.path_added {
            validate_terminal_path_removal(&config_paths)?;
            remove_terminal_path(&config_paths, &self.bin_dir())?;
        }
        if metadata.shim_path.exists() {
            fs::remove_file(&metadata.shim_path)
                .map_err(|error| format!("无法删除 {}：{error}", metadata.shim_path.display()))?;
        }
        for name in [METADATA_NAME, LAUNCHER_NAME, CONFIG_NAME] {
            let path = self.root.join(name);
            if path.exists() {
                fs::remove_file(&path)
                    .map_err(|error| format!("无法删除 {}：{error}", path.display()))?;
            }
        }
        Ok(metadata.command)
    }

    fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    fn read_metadata(&self) -> Option<InstallMetadata> {
        serde_json::from_reader(fs::File::open(self.root.join(METADATA_NAME)).ok()?).ok()
    }

    fn write_launcher_assets(&self) -> Result<(), String> {
        fs::create_dir_all(&self.root).map_err(|error| format!("无法创建终端命令目录：{error}"))?;
        let config = LauncherConfig {
            state_path: self.state.clone(),
            runtimes_path: self.runtimes.clone(),
            bundled_runtime: self.bundled_runtime.clone(),
        };
        fs::write(self.root.join(LAUNCHER_NAME), TERMINAL_LAUNCHER)
            .map_err(|error| format!("无法写入终端启动器：{error}"))?;
        fs::write(
            self.root.join(CONFIG_NAME),
            serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("无法写入终端运行时配置：{error}"))
    }

    fn write_shim(&self, path: &Path) -> Result<(), String> {
        #[cfg(windows)]
        let contents = format!(
            "@echo off\r\nrem {SHIM_MARKER}\r\nsetlocal\r\nset \"DSH_DESKTOP_TERMINAL_CWD=%CD%\"\r\npushd \"{}\" >nul || exit /b 1\r\n\"{}\" \"{LAUNCHER_NAME}\" %*\r\nset \"DSH_DESKTOP_EXIT=%ERRORLEVEL%\"\r\npopd >nul\r\nexit /b %DSH_DESKTOP_EXIT%\r\n",
            self.root.display(),
            self.node.display(),
        );
        #[cfg(not(windows))]
        let contents = format!(
            "#!/bin/sh\n# {SHIM_MARKER}\nexec {} {} \"$@\"\n",
            shell_quote(&self.node),
            shell_quote(&self.root.join(LAUNCHER_NAME)),
        );
        fs::write(path, contents)
            .map_err(|error| format!("无法写入 {}：{error}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("无法设置 {} 的执行权限：{error}", path.display()))?;
        }
        Ok(())
    }
}

struct PathChange {
    config_paths: Vec<PathBuf>,
    configured_shells: Vec<String>,
    added: bool,
}

fn choose_command_name(
    managed_command: Option<&str>,
    dsh_on_path: bool,
    dsh_target_exists: bool,
) -> &str {
    if let Some(command @ ("dsh" | "dsh-desktop")) = managed_command {
        return command;
    }
    if dsh_on_path || dsh_target_exists {
        "dsh-desktop"
    } else {
        "dsh"
    }
}

fn shim_path(bin_dir: &Path, command: &str) -> PathBuf {
    #[cfg(windows)]
    {
        bin_dir.join(format!("{command}.cmd"))
    }
    #[cfg(not(windows))]
    {
        bin_dir.join(command)
    }
}

fn is_managed_shim(path: &Path) -> bool {
    fs::read_to_string(path)
        .map(|contents| contents.contains(SHIM_MARKER))
        .unwrap_or(false)
}

struct CommandResolution {
    exists: bool,
    path: Option<PathBuf>,
}

fn resolve_command(name: &str) -> CommandResolution {
    #[cfg(windows)]
    {
        let mut command = Command::new("where.exe");
        command.creation_flags(CREATE_NO_WINDOW);
        let output = command.arg(name).stderr(Stdio::null()).output();
        let Ok(output) = output else {
            return CommandResolution {
                exists: false,
                path: None,
            };
        };
        if !output.status.success() {
            return CommandResolution {
                exists: false,
                path: None,
            };
        }
        let path = String::from_utf8_lossy(&output.stdout)
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .map(PathBuf::from);
        CommandResolution { exists: true, path }
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = env::var_os("PATH").and_then(|path| {
            env::split_paths(&path)
                .map(|directory| directory.join(name))
                .find(|candidate| {
                    candidate.metadata().is_ok_and(|metadata| {
                        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                    })
                })
        });
        CommandResolution {
            exists: path.is_some(),
            path,
        }
    }
}

#[cfg(windows)]
fn configure_terminal_path(
    bin_dir: &Path,
    _legacy_profile: Option<&Path>,
    path_owned_before: bool,
) -> Result<PathChange, String> {
    const SCRIPT: &str = r#"
$target = $env:DSH_DESKTOP_BIN
$current = [Environment]::GetEnvironmentVariable('Path', [EnvironmentVariableTarget]::User)
$parts = @($current -split ';' | Where-Object { $_ })
$present = $parts | Where-Object { [String]::Equals($_.TrimEnd('\\'), $target.TrimEnd('\\'), [StringComparison]::OrdinalIgnoreCase) }
if ($present) {
  Write-Output 'present'
} else {
  $updated = (@($parts) + $target) -join ';'
  [Environment]::SetEnvironmentVariable('Path', $updated, [EnvironmentVariableTarget]::User)
  Write-Output 'added'
}
"#;
    let mut command = Command::new("powershell.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("DSH_DESKTOP_BIN", bin_dir)
        .output()
        .map_err(|error| format!("无法更新当前用户 PATH：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "无法更新当前用户 PATH：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(PathChange {
        config_paths: Vec::new(),
        configured_shells: vec!["Windows".to_string()],
        added: path_owned_before || String::from_utf8_lossy(&output.stdout).contains("added"),
    })
}

#[cfg(not(windows))]
#[derive(Clone, Copy)]
enum ShellConfigKind {
    Posix,
    Fish,
}

#[cfg(not(windows))]
struct ShellConfigTarget {
    name: &'static str,
    path: PathBuf,
    kind: ShellConfigKind,
}

#[cfg(not(windows))]
fn configure_terminal_path(
    bin_dir: &Path,
    legacy_profile: Option<&Path>,
    path_owned_before: bool,
) -> Result<PathChange, String> {
    let home = user_home();
    let targets = vec![
        ShellConfigTarget {
            name: "zsh",
            path: zsh_config_path(&home),
            kind: ShellConfigKind::Posix,
        },
        ShellConfigTarget {
            name: "fish",
            path: fish_config_path(&home),
            kind: ShellConfigKind::Fish,
        },
    ];

    configure_shell_targets(bin_dir, targets, legacy_profile, path_owned_before)
}

#[cfg(not(windows))]
fn configure_shell_targets(
    bin_dir: &Path,
    targets: Vec<ShellConfigTarget>,
    legacy_profile: Option<&Path>,
    path_owned_before: bool,
) -> Result<PathChange, String> {
    for target in &targets {
        validate_shell_config(target)?;
    }

    for target in &targets {
        match target.kind {
            ShellConfigKind::Posix => write_posix_shell_config(&target.path, bin_dir)?,
            ShellConfigKind::Fish => write_fish_shell_config(&target.path, bin_dir)?,
        };
    }

    let config_paths = targets
        .iter()
        .map(|target| target.path.clone())
        .collect::<Vec<_>>();
    if let Some(profile) = legacy_profile {
        if !config_paths.iter().any(|path| paths_equal(path, profile)) {
            remove_managed_path_block(profile)?;
        }
    }

    Ok(PathChange {
        configured_shells: targets
            .iter()
            .map(|target| target.name.to_string())
            .collect(),
        added: path_owned_before || !config_paths.is_empty(),
        config_paths,
    })
}

#[cfg(not(windows))]
fn validate_shell_config(target: &ShellConfigTarget) -> Result<(), String> {
    let Ok(contents) = fs::read_to_string(&target.path) else {
        return Ok(());
    };
    match target.kind {
        ShellConfigKind::Posix => {
            let (starts, ends) = managed_path_marker_counts(&contents);
            if starts != ends {
                return Err(format!(
                    "{} 中的 DSH Desktop PATH 配置不完整，请先检查该文件。",
                    target.path.display()
                ));
            }
        }
        ShellConfigKind::Fish => {
            if !contents.starts_with(&format!("# {SHIM_MARKER}\n")) {
                return Err(format!(
                    "{} 已存在且不是由 DSH Desktop 创建，应用不会覆盖它。",
                    target.path.display()
                ));
            }
        }
    };
    Ok(())
}

#[cfg(windows)]
fn validate_terminal_path_removal(_config_paths: &[PathBuf]) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
fn validate_terminal_path_removal(config_paths: &[PathBuf]) -> Result<(), String> {
    for path in config_paths {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("无法读取 {}：{error}", path.display())),
        };
        if path
            .file_name()
            .is_some_and(|name| name == "dsh-desktop.fish")
        {
            if !contents.starts_with(&format!("# {SHIM_MARKER}\n")) {
                return Err(format!(
                    "{} 已被修改，DSH Desktop 不会删除它。",
                    path.display()
                ));
            }
        } else {
            let (starts, ends) = managed_path_marker_counts(&contents);
            if starts != ends {
                return Err(format!(
                    "{} 中的 DSH Desktop PATH 配置不完整，请先检查该文件。",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn write_posix_shell_config(profile: &Path, bin_dir: &Path) -> Result<bool, String> {
    let existing = match fs::read_to_string(profile) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("无法读取 {}：{error}", profile.display())),
    };
    let mut updated = remove_path_block(&existing);
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    let quoted_bin = shell_quote(bin_dir);
    updated.push_str(&format!(
        "{PATH_BLOCK_START}\ncase \":$PATH:\" in\n  *:{quoted_bin}:*) ;;\n  *) export PATH={quoted_bin}:\"$PATH\" ;;\nesac\n{PATH_BLOCK_END}\n"
    ));
    if updated == existing {
        return Ok(false);
    }
    if let Some(parent) = profile.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 {}：{error}", parent.display()))?;
    }
    fs::write(profile, updated)
        .map_err(|error| format!("无法更新 {}：{error}", profile.display()))?;
    Ok(true)
}

#[cfg(not(windows))]
fn write_fish_shell_config(profile: &Path, bin_dir: &Path) -> Result<bool, String> {
    let contents = format!(
        "# {SHIM_MARKER}\nif not contains -- {} $PATH\n    set -gx PATH {} $PATH\nend\n",
        fish_quote(bin_dir),
        fish_quote(bin_dir),
    );
    if fs::read_to_string(profile).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    if let Some(parent) = profile.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 {}：{error}", parent.display()))?;
    }
    fs::write(profile, contents)
        .map_err(|error| format!("无法更新 {}：{error}", profile.display()))?;
    Ok(true)
}

#[cfg(not(windows))]
fn zsh_config_path(home: &Path) -> PathBuf {
    if let Some(dot_dir) = env::var_os("ZDOTDIR").filter(|value| !value.is_empty()) {
        return resolve_config_dir(home, PathBuf::from(dot_dir)).join(".zshrc");
    }
    if Path::new("/bin/zsh").is_file() {
        if let Ok(output) = Command::new("/bin/zsh")
            .args(["-c", "printf '__DSH_DESKTOP__%s\\n' \"${ZDOTDIR:-$HOME}\""])
            .env("HOME", home)
            .stderr(Stdio::null())
            .output()
        {
            if let Some(value) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find_map(|line| line.strip_prefix("__DSH_DESKTOP__"))
                .filter(|value| !value.is_empty())
            {
                return resolve_config_dir(home, PathBuf::from(value)).join(".zshrc");
            }
        }
    }
    home.join(".zshrc")
}

#[cfg(not(windows))]
fn fish_config_path(home: &Path) -> PathBuf {
    fish_config_dir(home).join("conf.d/dsh-desktop.fish")
}

#[cfg(not(windows))]
fn fish_config_dir(home: &Path) -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| resolve_config_dir(home, path))
        .unwrap_or_else(|| home.join(".config"))
        .join("fish")
}

#[cfg(not(windows))]
fn resolve_config_dir(home: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

#[cfg(windows)]
fn remove_terminal_path(_config_paths: &[PathBuf], bin_dir: &Path) -> Result<(), String> {
    const SCRIPT: &str = r#"
$target = $env:DSH_DESKTOP_BIN
$current = [Environment]::GetEnvironmentVariable('Path', [EnvironmentVariableTarget]::User)
$parts = @($current -split ';' | Where-Object { $_ -and -not [String]::Equals($_.TrimEnd('\\'), $target.TrimEnd('\\'), [StringComparison]::OrdinalIgnoreCase) })
[Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), [EnvironmentVariableTarget]::User)
"#;
    let mut command = Command::new("powershell.exe");
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("DSH_DESKTOP_BIN", bin_dir)
        .output()
        .map_err(|error| format!("无法恢复当前用户 PATH：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "无法恢复当前用户 PATH：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(not(windows))]
fn remove_terminal_path(config_paths: &[PathBuf], _bin_dir: &Path) -> Result<(), String> {
    for path in config_paths {
        if path
            .file_name()
            .is_some_and(|name| name == "dsh-desktop.fish")
        {
            if !path.exists() {
                continue;
            }
            let contents = fs::read_to_string(path)
                .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
            if !contents.starts_with(&format!("# {SHIM_MARKER}\n")) {
                return Err(format!(
                    "{} 已被修改，DSH Desktop 不会删除它。",
                    path.display()
                ));
            }
            fs::remove_file(path)
                .map_err(|error| format!("无法删除 {}：{error}", path.display()))?;
        } else {
            remove_managed_path_block(path)?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_managed_path_block(profile: &Path) -> Result<(), String> {
    if !profile.exists() {
        return Ok(());
    }
    let contents = fs::read_to_string(profile)
        .map_err(|error| format!("无法读取 {}：{error}", profile.display()))?;
    let updated = remove_path_block(&contents);
    if updated != contents {
        fs::write(profile, updated)
            .map_err(|error| format!("无法更新 {}：{error}", profile.display()))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn remove_path_block(contents: &str) -> String {
    let mut updated = contents.to_string();
    while let Some(start) = updated.find(PATH_BLOCK_START) {
        let Some(relative_end) = updated[start..].find(PATH_BLOCK_END) else {
            break;
        };
        let mut end = start + relative_end + PATH_BLOCK_END.len();
        if updated[end..].starts_with('\n') {
            end += 1;
        }
        updated.replace_range(start..end, "");
    }
    updated
}

#[cfg(not(windows))]
fn managed_path_marker_counts(contents: &str) -> (usize, usize) {
    (
        contents.matches(PATH_BLOCK_START).count(),
        contents.matches(PATH_BLOCK_END).count(),
    )
}

#[cfg(not(windows))]
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

#[cfg(not(windows))]
fn fish_quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
    )
}

#[cfg(not(windows))]
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

fn paths_equal(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['\\', '/']))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_an_existing_dsh_command_name() {
        assert_eq!(choose_command_name(None, true, false), "dsh-desktop");
        assert_eq!(choose_command_name(None, false, true), "dsh-desktop");
        assert_eq!(choose_command_name(None, false, false), "dsh");
        assert_eq!(choose_command_name(Some("dsh"), true, true), "dsh");
    }

    #[cfg(not(windows))]
    #[test]
    fn removes_only_the_managed_profile_block() {
        let input = format!(
            "export EDITOR=vim\n{PATH_BLOCK_START}\nexport PATH=\"$HOME/.local/bin:$PATH\"\n{PATH_BLOCK_END}\nalias ll='ls -l'\n"
        );
        assert_eq!(
            remove_path_block(&input),
            "export EDITOR=vim\nalias ll='ls -l'\n"
        );
    }

    #[test]
    fn reads_legacy_terminal_command_metadata() {
        let metadata: InstallMetadata = serde_json::from_str(
            r#"{
  "command": "dsh",
  "shimPath": "/tmp/dsh",
  "profilePath": "/tmp/.zprofile",
  "pathAdded": true
}"#,
        )
        .expect("read legacy metadata");
        assert_eq!(metadata.profile_path, Some(PathBuf::from("/tmp/.zprofile")));
        assert!(metadata.shell_config_paths.is_empty());
    }

    #[cfg(not(windows))]
    #[test]
    fn configures_zsh_and_fish_and_migrates_the_legacy_profile() {
        use std::os::unix::fs::PermissionsExt;

        let root = env::temp_dir().join(format!(
            "dsh desktop shell config test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let bin_dir = root.join("application support/terminal/bin");
        fs::create_dir_all(&bin_dir).expect("create command directory");
        let shim = bin_dir.join("dsh");
        fs::write(&shim, "#!/bin/sh\nprintf 'test-dsh\\n'\n").expect("write test command");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))
            .expect("make test command executable");

        let legacy_profile = home.join(".zprofile");
        fs::create_dir_all(&home).expect("create test home");
        fs::write(
            &legacy_profile,
            format!(
                "export EDITOR=vim\n{PATH_BLOCK_START}\nexport PATH=/old/path:\"$PATH\"\n{PATH_BLOCK_END}\n"
            ),
        )
        .expect("write legacy profile");
        let zsh_config = home.join(".zshrc");
        let fish_config = home.join(".config/fish/conf.d/dsh-desktop.fish");
        let result = configure_shell_targets(
            &bin_dir,
            vec![
                ShellConfigTarget {
                    name: "zsh",
                    path: zsh_config.clone(),
                    kind: ShellConfigKind::Posix,
                },
                ShellConfigTarget {
                    name: "fish",
                    path: fish_config.clone(),
                    kind: ShellConfigKind::Fish,
                },
            ],
            Some(&legacy_profile),
            true,
        )
        .expect("configure shell paths");

        assert_eq!(result.configured_shells, ["zsh", "fish"]);
        assert_eq!(
            fs::read_to_string(&legacy_profile).expect("read migrated profile"),
            "export EDITOR=vim\n"
        );
        assert!(fs::read_to_string(&zsh_config)
            .expect("read zsh config")
            .contains(PATH_BLOCK_START));
        assert!(fs::read_to_string(&fish_config)
            .expect("read fish config")
            .contains(SHIM_MARKER));

        if Path::new("/bin/zsh").is_file() {
            let output = Command::new("/bin/zsh")
                .args(["-ic", "command -v dsh"])
                .env_clear()
                .env("HOME", &home)
                .env("ZDOTDIR", &home)
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .output()
                .expect("run clean zsh");
            assert!(
                output.status.success(),
                "zsh did not load the managed path: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                shim.display().to_string()
            );
        }

        let fish = [
            "/opt/homebrew/bin/fish",
            "/usr/local/bin/fish",
            "/usr/bin/fish",
        ]
        .into_iter()
        .map(Path::new)
        .find(|path| path.is_file());
        if let Some(fish) = fish {
            let output = Command::new(fish)
                .args(["-ic", "command -v dsh"])
                .env_clear()
                .env("HOME", &home)
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
                .output()
                .expect("run clean fish");
            assert!(
                output.status.success(),
                "fish did not load the managed path: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                shim.display().to_string()
            );
        }

        remove_terminal_path(&result.config_paths, &bin_dir).expect("remove shell paths");
        assert!(!fish_config.exists());
        assert!(!fs::read_to_string(&zsh_config)
            .expect("read cleaned zsh config")
            .contains(PATH_BLOCK_START));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_launcher_runs_the_bundled_dsh() {
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
        let manifest: serde_json::Value = serde_json::from_reader(
            fs::File::open(resources.join("runtime-manifest.json"))
                .expect("open prepared runtime manifest"),
        )
        .expect("parse prepared runtime manifest");
        let version = manifest["dshVersion"]
            .as_str()
            .expect("runtime manifest dsh version")
            .to_string();
        let node = resources.join(if cfg!(windows) {
            "node/node.exe"
        } else {
            "node/bin/node"
        });
        let root = env::temp_dir().join(format!(
            "dsh-desktop-terminal-launcher-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create terminal launcher test directory");
        let manager = TerminalCommandManager::new(
            &root,
            node.clone(),
            resources.join("bootstrap-runtime"),
            root.join("runtime/versions"),
            root.join("runtime/state.json"),
        );
        manager
            .write_launcher_assets()
            .expect("write terminal launcher assets");

        let output = Command::new(node)
            .arg(LAUNCHER_NAME)
            .arg("--version")
            .current_dir(root.join("terminal"))
            .output()
            .expect("run terminal launcher");
        let _ = fs::remove_dir_all(&root);
        assert!(
            output.status.success(),
            "terminal launcher failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), version);
    }
}
