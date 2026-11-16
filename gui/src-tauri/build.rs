//! Tauri's build step: reads `tauri.conf.json`, and declares the shell's two commands so the
//! window's capability can grant exactly them (ADR 0023).
fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&["core_send", "core_subscribe"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("the Tauri build step failed");
}
