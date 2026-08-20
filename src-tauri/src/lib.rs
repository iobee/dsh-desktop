mod app_updater;
mod runtime;
mod terminal_command;
#[cfg(target_os = "macos")]
mod window_chrome;

use std::{env, sync::Arc, thread};

use app_updater::AppUpdater;
use runtime::RuntimeManager;
use serde::Serialize;
use tauri::{
    menu::{MenuBuilder, SubmenuBuilder},
    Manager, RunEvent, State,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

struct AppState {
    runtime: Arc<RuntimeManager>,
    updater: Arc<AppUpdater>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AboutInfo {
    desktop_version: String,
    runtime: runtime::RuntimeSnapshot,
    desktop_update: app_updater::AppUpdateSnapshot,
}

#[tauri::command]
async fn bootstrap(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<runtime::StartupInfo, String> {
    let manager = Arc::clone(&state.runtime);
    let start_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || manager.start(&start_app))
        .await
        .map_err(|error| format!("启动任务异常结束：{error}"))??;
    state.runtime.schedule_update_check(app.clone());
    state.updater.schedule_update_check(app);
    Ok(result)
}

#[tauri::command]
fn open_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.runtime.open_logs()
}

#[tauri::command]
fn get_about_info(app: tauri::AppHandle, state: State<'_, AppState>) -> AboutInfo {
    AboutInfo {
        desktop_version: app.package_info().version.to_string(),
        runtime: state.runtime.snapshot(),
        desktop_update: state.updater.snapshot(),
    }
}

#[tauri::command]
fn check_dsh_update(app: tauri::AppHandle, state: State<'_, AppState>) -> bool {
    state.runtime.check_for_updates(app, true)
}

#[tauri::command]
fn set_dsh_update_channel(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    channel: runtime::RuntimeUpdateChannel,
) -> Result<bool, String> {
    if !state.runtime.set_update_channel(channel)? {
        return Ok(false);
    }
    Ok(state.runtime.check_for_updates(app, true))
}

#[tauri::command]
fn check_app_update(app: tauri::AppHandle, state: State<'_, AppState>) -> bool {
    state.updater.check_for_updates(app, true)
}

fn show_configured_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == label)
        .ok_or_else(|| format!("找不到 {label} 窗口配置"))?;
    let window = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|error| error.to_string())?
        .build()
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let _ = window_chrome::hide_traffic_lights(&window);
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    Ok(())
}

fn install_menu(app: &tauri::App) -> tauri::Result<()> {
    let app_menu = SubmenuBuilder::new(app, "DSH Desktop")
        .text("about", "关于 DSH Desktop")
        .text("updates", "检查更新…")
        .separator()
        .text("restart", "重新启动")
        .separator()
        .text("install-terminal-command", "安装终端命令…")
        .text("uninstall-terminal-command", "移除终端命令…")
        .separator()
        .text("open-logs", "打开日志")
        .separator()
        .quit()
        .build()?;
    let edit_menu = SubmenuBuilder::new(app, "编辑")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window_menu = SubmenuBuilder::new(app, "窗口")
        .minimize()
        .close_window()
        .build()?;
    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &edit_menu, &window_menu])
        .build()?;
    app.set_menu(menu)?;
    Ok(())
}

fn install_terminal_command(app: &tauri::AppHandle, runtime: Arc<RuntimeManager>) {
    let handle = app.clone();
    app.dialog()
        .message(
            "这会安装一个终端命令，并同时为 zsh 与 fish 配置 PATH。\n\n如果系统里已经有 dsh，DSH Desktop 会改用 dsh-desktop，不会覆盖或改变原命令的优先级。",
        )
        .title("安装终端命令")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "安装".to_string(),
            "取消".to_string(),
        ))
        .show(move |confirmed| {
            if !confirmed {
                return;
            }
            thread::spawn(move || {
                let result = runtime.install_terminal_command();
                let (message, kind) = match result {
                    Ok(installed) => {
                        let shell_note = if installed.configured_shells.is_empty() {
                            "请关闭并重新打开终端后使用。".to_string()
                        } else {
                            format!(
                                "已配置：{}。请打开新的终端会话后使用。",
                                installed.configured_shells.join("、")
                            )
                        };
                        (
                            format!(
                                "已安装命令：{}\n位置：{}\n\n{}",
                                installed.command,
                                installed.shim_path.display(),
                                shell_note
                            ),
                            MessageDialogKind::Info,
                        )
                    }
                    Err(error) => (error, MessageDialogKind::Error),
                };
                handle
                    .dialog()
                    .message(message)
                    .title("终端命令")
                    .kind(kind)
                    .show(|_| {});
            });
        });
}

fn uninstall_terminal_command(app: &tauri::AppHandle, runtime: Arc<RuntimeManager>) {
    let handle = app.clone();
    app.dialog()
        .message("只会移除 DSH Desktop 自己安装的命令和 PATH 配置，不会触碰其他 dsh。")
        .title("移除终端命令")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "移除".to_string(),
            "取消".to_string(),
        ))
        .show(move |confirmed| {
            if !confirmed {
                return;
            }
            thread::spawn(move || {
                let result = runtime.uninstall_terminal_command();
                let (message, kind) = match result {
                    Ok(command) => (
                        format!("已移除 {command}。重新打开终端后生效。"),
                        MessageDialogKind::Info,
                    ),
                    Err(error) => (error, MessageDialogKind::Error),
                };
                handle
                    .dialog()
                    .message(message)
                    .title("终端命令")
                    .kind(kind)
                    .show(|_| {});
            });
        });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(window_chrome::plugin());
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let resource_dir = app.path().resource_dir()?;
            let app_data_dir = app.path().app_data_dir()?;
            let runtime = Arc::new(RuntimeManager::new(resource_dir, app_data_dir.clone())?);
            if env::var_os("DSH_DESKTOP_RUNTIME_SMOKE").is_some() {
                let info = runtime
                    .smoke_bundled_runtime()
                    .map_err(std::io::Error::other)?;
                println!(
                    "packaged runtime smoke passed: dsh {} at {}",
                    info.dsh_version, info.url
                );
                std::process::exit(0);
            }
            let updater = Arc::new(AppUpdater::new(app_data_dir)?);
            app.manage(AppState { runtime, updater });
            install_menu(app)?;
            #[cfg(target_os = "macos")]
            {
                let window = app.get_webview_window("main").ok_or_else(|| {
                    std::io::Error::other("main window is unavailable during setup")
                })?;
                window_chrome::hide_traffic_lights(&window)?;
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            let state = app.state::<AppState>();
            match event.id().as_ref() {
                "about" => {
                    if let Err(error) = show_configured_window(app, "about") {
                        eprintln!("failed to show About window: {error}");
                    }
                }
                "updates" => {
                    if let Err(error) = show_configured_window(app, "updates") {
                        eprintln!("failed to show Updates window: {error}");
                    }
                }
                "restart" => {
                    state.updater.terminate();
                    state.runtime.terminate();
                    app.restart();
                }
                "install-terminal-command" => {
                    install_terminal_command(app, Arc::clone(&state.runtime));
                }
                "uninstall-terminal-command" => {
                    uninstall_terminal_command(app, Arc::clone(&state.runtime));
                }
                "open-logs" => {
                    let _ = state.runtime.open_logs();
                }
                _ => {}
            }
        })
        .on_window_event(|window, event| {
            if matches!(window.label(), "about" | "updates") {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                let app = window.app_handle();
                let state = app.state::<AppState>();
                state.updater.terminate();
                state.runtime.terminate();
                app.exit(0);
            }
        });
    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bootstrap,
        open_logs,
        get_about_info,
        check_dsh_update,
        set_dsh_update_channel,
        check_app_update,
        window_chrome::set_traffic_lights_visible
    ]);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        bootstrap,
        open_logs,
        get_about_info,
        check_dsh_update,
        set_dsh_update_channel,
        check_app_update
    ]);
    let app = builder
        .build(tauri::generate_context!())
        .expect("failed to build DSH Desktop");

    app.run(|app, event| {
        if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
            let state = app.state::<AppState>();
            state.updater.terminate();
            state.runtime.terminate();
        }
    });
}
