fn main() {
    const COMMANDS: &[&str] = &[
        "start_mock_stream",
        "ack_stream",
        "cancel_stream",
        "get_stream_snapshot",
        "sanitize_plugin_html",
        "privileged_probe",
        "privileged_probe_count",
    ];

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to build LorePia Tauri command permissions");
}
