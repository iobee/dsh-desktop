use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

use crate::AppState;

const UPDATE_DELAY: Duration = Duration::from_secs(120);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const UPDATE_INTERVAL_SECS: u64 = 24 * 60 * 60;
const FAILED_RETRY_SECS: u64 = 60 * 60;

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateState {
    last_update_attempt: Option<u64>,
    last_successful_check: Option<u64>,
}

pub struct AppUpdater {
    state_path: PathBuf,
    log_path: PathBuf,
    state: Mutex<UpdateState>,
    check_scheduled: AtomicBool,
    check_running: AtomicBool,
    shutting_down: AtomicBool,
}

impl AppUpdater {
    pub fn new(app_data_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let logs = app_data_dir.join("logs");
        fs::create_dir_all(&logs)?;
        let state_path = app_data_dir.join("app-update-state.json");
        let state = read_state(&state_path).unwrap_or_default();
        Ok(Self {
            state_path,
            log_path: logs.join("desktop.log"),
            state: Mutex::new(state),
            check_scheduled: AtomicBool::new(false),
            check_running: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
        })
    }

    pub fn schedule_update_check(self: &Arc<Self>, app: tauri::AppHandle) {
        if self.check_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let updater = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(UPDATE_DELAY);
            if !updater.shutting_down.load(Ordering::Acquire) {
                updater.check_for_updates(app, false);
            }
        });
    }

    pub fn check_for_updates(self: &Arc<Self>, app: tauri::AppHandle, force: bool) {
        if self.check_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let updater = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if let Err(error) = updater.check_for_updates_inner(app.clone(), force).await {
                append_log(
                    &updater.log_path,
                    &format!("DSH Desktop update check failed: {error}"),
                );
                if force && !updater.shutting_down.load(Ordering::Acquire) {
                    show_message(
                        &app,
                        "检查更新失败",
                        &format!("暂时无法检查 DSH Desktop 更新。\n\n{error}"),
                        MessageDialogKind::Error,
                    );
                }
            }
            updater.check_running.store(false, Ordering::Release);
        });
    }

    pub fn terminate(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    async fn check_for_updates_inner(
        &self,
        app: tauri::AppHandle,
        force: bool,
    ) -> Result<(), String> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Ok(());
        }

        let now = unix_time();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "应用更新状态锁已损坏".to_string())?;
            if !force && !update_is_due(&state, now) {
                return Ok(());
            }
            state.last_update_attempt = Some(now);
            self.save_state(&state)?;
        }

        append_log(&self.log_path, "checking GitHub for a DSH Desktop update");
        let update = app
            .updater_builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| format!("无法初始化更新器：{error}"))?
            .check()
            .await
            .map_err(|error| format!("GitHub 更新检查失败：{error}"))?;

        self.mark_successful_check()?;
        let Some(update) = update else {
            append_log(&self.log_path, "DSH Desktop is already current");
            if force {
                show_message(
                    &app,
                    "已经是最新版",
                    "当前 DSH Desktop 已是最新版。",
                    MessageDialogKind::Info,
                );
            }
            return Ok(());
        };

        let version = update.version.clone();
        append_log(
            &self.log_path,
            &format!("DSH Desktop {version} is available"),
        );
        let accepted = app
            .dialog()
            .message(format!(
                "发现 DSH Desktop {version}。\n\n更新包会先经过签名验证，安装完成后应用将自动重启。"
            ))
            .title("DSH Desktop 更新")
            .buttons(MessageDialogButtons::OkCancelCustom(
                "下载并安装".to_string(),
                "稍后".to_string(),
            ))
            .blocking_show();
        if !accepted || self.shutting_down.load(Ordering::Acquire) {
            append_log(&self.log_path, &format!("DSH Desktop {version} deferred"));
            return Ok(());
        }

        if let Some(window) = app.get_webview_window("main") {
            let _ = window.set_title(&format!("DSH Desktop · 正在下载 {version}"));
        }
        append_log(
            &self.log_path,
            &format!("downloading signed DSH Desktop {version} update"),
        );
        let log_path = self.log_path.clone();
        let finished_log = self.log_path.clone();
        let mut downloaded = 0_u64;
        let mut next_percentage = 25_u64;
        update
            .download_and_install(
                move |chunk_length, content_length| {
                    downloaded = downloaded.saturating_add(chunk_length as u64);
                    if let Some(total) = content_length.filter(|total| *total > 0) {
                        let percentage = downloaded.saturating_mul(100) / total;
                        if percentage >= next_percentage {
                            append_log(&log_path, &format!("app update download: {percentage}%"));
                            next_percentage = next_percentage.saturating_add(25);
                        }
                    }
                },
                move || append_log(&finished_log, "app update download finished"),
            )
            .await
            .map_err(|error| format!("下载或安装更新失败：{error}"))?;

        append_log(
            &self.log_path,
            &format!("DSH Desktop {version} installed; restarting"),
        );
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.set_title(&format!("DSH Desktop · {version} 已安装，正在重启"));
        }
        let state = app.state::<AppState>();
        state.runtime.terminate();
        self.terminate();
        app.restart();
    }

    fn mark_successful_check(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "应用更新状态锁已损坏".to_string())?;
        state.last_successful_check = Some(unix_time());
        self.save_state(&state)
    }

    fn save_state(&self, state: &UpdateState) -> Result<(), String> {
        let temporary = self.state_path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| format!("无法编码应用更新状态：{error}"))?;
        fs::write(&temporary, bytes).map_err(|error| format!("无法写入应用更新状态：{error}"))?;
        fs::rename(&temporary, &self.state_path)
            .map_err(|error| format!("无法提交应用更新状态：{error}"))
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

fn read_state(path: &Path) -> Option<UpdateState> {
    serde_json::from_reader(File::open(path).ok()?).ok()
}

fn append_log(path: &Path, message: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{}] {message}", unix_time());
    }
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn update_is_due(state: &UpdateState, now: u64) -> bool {
    let success_due = state
        .last_successful_check
        .is_none_or(|last| now.saturating_sub(last) >= UPDATE_INTERVAL_SECS);
    let retry_due = state
        .last_update_attempt
        .is_none_or(|last| now.saturating_sub(last) >= FAILED_RETRY_SECS);
    success_due && retry_due
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_daily_and_retries_failures_after_one_hour() {
        let now = 200_000;
        assert!(update_is_due(&UpdateState::default(), now));
        assert!(!update_is_due(
            &UpdateState {
                last_update_attempt: Some(now - 30),
                ..UpdateState::default()
            },
            now
        ));
        assert!(!update_is_due(
            &UpdateState {
                last_update_attempt: Some(now - FAILED_RETRY_SECS),
                last_successful_check: Some(now - 60),
            },
            now
        ));
        assert!(update_is_due(
            &UpdateState {
                last_update_attempt: Some(now - FAILED_RETRY_SECS),
                last_successful_check: Some(now - UPDATE_INTERVAL_SECS),
            },
            now
        ));
    }
}
