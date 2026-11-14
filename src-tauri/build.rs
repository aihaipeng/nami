fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_snapshot",
            "translation_run",
            "translation_cancel",
            "translation_engine_set",
            "selection_take",
            "window_show",
            "window_hide",
            "window_pin",
            "history_visibility_set",
            "history_toggle",
            "history_width_set",
            "translation_chrome_update",
            "history_sample_select",
            "history_snapshot",
            "history_record_update",
            "translation_columns_set",
            "translation_sample_copy",
            "clipboard_record_use",
            "locale_set",
            "permission_request",
        ]),
    ))
    .expect("failed to build Nami manifest");
}
