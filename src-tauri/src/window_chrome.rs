use std::error::Error;

use objc2_app_kit::{NSWindow, NSWindowButton};
use tauri::WebviewWindow;

pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("window-chrome")
        .js_init_script(include_str!("window_chrome.js"))
        .build()
}

pub fn hide_traffic_lights(window: &WebviewWindow) -> Result<(), Box<dyn Error>> {
    set_native_traffic_lights_visible(window, false)
}

#[tauri::command]
pub fn set_traffic_lights_visible(window: WebviewWindow, visible: bool) -> Result<(), String> {
    set_native_traffic_lights_visible(&window, visible).map_err(|error| error.to_string())
}

fn set_native_traffic_lights_visible(
    window: &WebviewWindow,
    visible: bool,
) -> Result<(), Box<dyn Error>> {
    let ns_window_pointer = window.ns_window()?.cast::<NSWindow>();
    // SAFETY: Tauri owns the NSWindow for the lifetime of its WebviewWindow.
    let ns_window = unsafe { &*ns_window_pointer };
    for button_type in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton(button_type) {
            button.setHidden(!visible);
        }
    }
    Ok(())
}
