fn main() {
    let app_manifest = tauri_build::AppManifest::new().commands(&[
        "bootstrap",
        "open_logs",
        "set_traffic_lights_visible",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(app_manifest))
        .expect("failed to build Tauri application metadata");
}
