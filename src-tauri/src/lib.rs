mod runtime;

use std::sync::Arc;

use runtime::RuntimeManager;
use tauri::{
    menu::{MenuBuilder, SubmenuBuilder},
    Manager, RunEvent, State,
};

struct AppState {
    runtime: Arc<RuntimeManager>,
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
    state.runtime.schedule_update_check(app);
    Ok(result)
}

#[tauri::command]
fn open_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.runtime.open_logs()
}

fn install_menu(app: &tauri::App) -> tauri::Result<()> {
    let app_menu = SubmenuBuilder::new(app, "DSH Desktop")
        .text("check-update", "检查 dsh 更新…")
        .text("restart", "重新启动")
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let resource_dir = app.path().resource_dir()?;
            let app_data_dir = app.path().app_data_dir()?;
            let runtime = Arc::new(RuntimeManager::new(resource_dir, app_data_dir)?);
            app.manage(AppState { runtime });
            install_menu(app)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            let state = app.state::<AppState>();
            match event.id().as_ref() {
                "check-update" => state.runtime.check_for_updates(app.clone(), true),
                "restart" => {
                    state.runtime.terminate();
                    app.restart();
                }
                "open-logs" => {
                    let _ = state.runtime.open_logs();
                }
                _ => {}
            }
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let app = window.app_handle();
                app.state::<AppState>().runtime.terminate();
                app.exit(0);
            }
        })
        .invoke_handler(tauri::generate_handler![bootstrap, open_logs])
        .build(tauri::generate_context!())
        .expect("failed to build DSH Desktop");

    app.run(|app, event| {
        if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
            app.state::<AppState>().runtime.terminate();
        }
    });
}
