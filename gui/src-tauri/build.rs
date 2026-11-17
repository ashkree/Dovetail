//! Tauri's build step: reads `tauri.conf.json`, and declares the shell's two commands so the
//! window's capability can grant exactly them (ADR 0023).
fn main() {
    // The perf build's measurements go out through one more command (scripts/perf-gui.sh).
    let commands: &[&str] = if std::env::var_os("CARGO_FEATURE_PERF").is_some() {
        &["core_send", "core_subscribe", "perf_log"]
    } else {
        &["core_send", "core_subscribe"]
    };
    let manifest = tauri_build::AppManifest::new().commands(commands);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("the Tauri build step failed");
}
