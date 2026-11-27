fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=ServiceManagement");
        if std::env::var_os("CARGO_FEATURE_PORTABLE_UI").is_none() {
            build_apple_translation_bridge();
        }
    }

    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_snapshot",
            "translation_run",
            "translation_cancel",
            "translation_copy",
            "translation_engine_set",
            "translation_language_settings_open",
            "translation_preferences_set",
            "translation_shortcut_set",
            "translation_shortcuts_restore",
            "translation_shortcut_capture",
            "selection_take",
            "selection_present",
            "window_show",
            "window_hide",
            "window_pin",
            "records_details_toggle",
            "clipboard_details_select",
            "clipboard_note_open",
            "records_editor_set",
            "records_navigation_resolve",
            "clipboard_width_set",
            "clipboard_detail_width_set",
            "clipboard_translation_close",
            "clipboard_translation_clear_record",
            "translation_chrome_update",
            "translation_columns_set",
            "clipboard_record_use",
            "clipboard_query",
            "clipboard_record_detail",
            "clipboard_image_qr_codes",
            "clipboard_open_url",
            "clipboard_open_target",
            "clipboard_record_has_text",
            "clipboard_record_drag",
            "clipboard_record_process",
            "clipboard_translation_take",
            "clipboard_record_update",
            "clipboard_capture_set",
            "clipboard_recording_type_set",
            "clipboard_options_get",
            "clipboard_excluded_app_pick",
            "clipboard_excluded_app_remove",
            "clipboard_ignored_type_add",
            "clipboard_ignored_type_update",
            "clipboard_ignored_type_remove",
            "clipboard_ignored_regex_add",
            "clipboard_ignored_regex_remove",
            "clipboard_ignore_secrets_set",
            "clipboard_behavior_set",
            "clipboard_search_set",
            "records_open_focus_set",
            "records_preview_lines_set",
            "clipboard_record_shortcut_set",
            "records_storage_info",
            "records_cleanup_preview",
            "records_cleanup_apply",
            "clipboard_shortcut_set",
            "clipboard_image",
            "clipboard_files",
            "locale_set",
            "window_position_mode_set",
            "launch_at_login_set",
            "backup_export",
            "backup_import_pick",
            "backup_import_unlock",
            "backup_import_apply",
            "permission_request",
        ]),
    ))
    .expect("failed to build Nami manifest");
}

fn build_apple_translation_bridge() {
    use std::path::PathBuf;
    use std::process::Command;

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let swift_file = manifest_dir.join("swift/AppleTranslationBridge.swift");
    println!("cargo:rerun-if-changed={}", swift_file.display());

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let object_file = out_dir.join("AppleTranslationBridge.o");
    let archive_file = out_dir.join("libnami_apple_translation.a");

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    let target_triple = match arch.as_str() {
        "aarch64" => "arm64-apple-macos13.0",
        "x86_64" => "x86_64-apple-macos13.0",
        other => panic!("unsupported macOS target arch for Apple Translation bridge: {other}"),
    };

    let sdk_output = Command::new("xcrun")
        .args(["--sdk", "macosx", "--show-sdk-path"])
        .output()
        .expect("failed to query macOS SDK path via xcrun");
    assert!(
        sdk_output.status.success(),
        "xcrun --show-sdk-path failed: {}",
        String::from_utf8_lossy(&sdk_output.stderr)
    );
    let sdk_path = String::from_utf8(sdk_output.stdout)
        .expect("invalid UTF-8 in macOS SDK path")
        .trim()
        .to_string();

    let swiftc_output = Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .expect("failed to locate swiftc via xcrun");
    assert!(
        swiftc_output.status.success(),
        "xcrun --find swiftc failed: {}",
        String::from_utf8_lossy(&swiftc_output.stderr)
    );
    let swiftc_path = PathBuf::from(
        String::from_utf8(swiftc_output.stdout)
            .expect("invalid UTF-8 in swiftc path")
            .trim(),
    );

    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let opt_flag = if profile == "release" { "-O" } else { "-Onone" };

    let compile_status = Command::new(&swiftc_path)
        .args([
            opt_flag,
            "-c",
            "-parse-as-library",
            "-target",
            target_triple,
            "-sdk",
            &sdk_path,
        ])
        .arg(&swift_file)
        .arg("-o")
        .arg(&object_file)
        .status()
        .expect("failed to invoke swiftc for AppleTranslationBridge.swift");
    assert!(
        compile_status.success(),
        "swiftc failed compiling AppleTranslationBridge.swift"
    );

    let archive_status = Command::new("ar")
        .args(["rcs"])
        .arg(&archive_file)
        .arg(&object_file)
        .status()
        .expect("failed to archive libnami_apple_translation.a");
    assert!(
        archive_status.success(),
        "ar failed creating libnami_apple_translation.a"
    );

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=nami_apple_translation");
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    if let Some(swift_lib_dir) = swiftc_path
        .parent()
        .and_then(|bin| bin.parent())
        .map(|usr| usr.join("lib/swift/macosx"))
    {
        if swift_lib_dir.is_dir() {
            println!("cargo:rustc-link-search=native={}", swift_lib_dir.display());
        }
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,Translation");
    println!("cargo:rustc-link-arg=-Wl,-weak_framework,_Translation_SwiftUI");
}
