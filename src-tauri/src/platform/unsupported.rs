// Ordinary translation/history windows work before OS integrations are implemented.
const UNAVAILABLE: &str = "platform_unavailable";

#[derive(Clone, Copy, Debug)]
pub struct Target;

pub struct CopySnapshot {
    pub version: i64,
    pub initial_text: Option<String>,
    pub initial_has_items: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionRead {
    pub version: i64,
    pub text: Option<String>,
    pub has_items: bool,
}

// The portable host keeps the native selection API surface even though it
// never constructs these variants.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualAccessibility {
    Enabled,
    AlreadyEnabled,
    Unsupported,
}

pub fn begin_selection_copy(_: Option<Target>) -> Result<CopySnapshot, &'static str> {
    Err(UNAVAILABLE)
}
pub fn read_selection_payload() -> SelectionRead {
    SelectionRead {
        version: -1,
        text: None,
        has_items: false,
    }
}
pub fn finish_selection_copy(
    _: &CopySnapshot,
    _: String,
    _: i64,
) -> Result<(String, i64), &'static str> {
    Err(UNAVAILABLE)
}

pub fn activation_epoch() -> u64 {
    0
}
pub fn capture_target() -> Option<Target> {
    None
}
pub fn capture_selection_target() -> Option<Target> {
    None
}
pub fn target_app_info(_: Target) -> Option<serde_json::Value> {
    None
}
pub fn current_apple_event() -> Option<serde_json::Value> {
    None
}
pub fn history_width() -> Option<f64> {
    None
}
pub fn save_history_width(_: f64) {}
pub fn accessibility() -> bool {
    false
}
pub fn launch_at_login_enabled() -> bool {
    false
}
pub fn set_launch_at_login(_: bool) -> Result<bool, String> {
    Err(UNAVAILABLE.into())
}
pub fn request_accessibility() -> bool {
    false
}
pub fn selected_text(_: Option<Target>) -> Result<String, &'static str> {
    Err(UNAVAILABLE)
}
pub fn enable_selection_accessibility(_: Target) -> Result<ManualAccessibility, &'static str> {
    Err(UNAVAILABLE)
}
pub fn wait_for_ax_ready(_: Target) -> bool {
    false
}
#[allow(dead_code)]
pub fn secure_input() -> bool {
    false
}
pub fn activate(_: Target) -> bool {
    false
}
pub fn target_alive(_: Target) -> bool {
    false
}
pub fn target_focused(_: Target) -> bool {
    false
}
pub fn clipboard_version() -> i64 {
    0
}
pub fn write_text(_: &str) -> Result<i64, (&'static str, Option<i64>)> {
    Err((UNAVAILABLE, None))
}
pub fn paste_keys_down(_: Target, _: i64) -> Result<(), &'static str> {
    Err(UNAVAILABLE)
}
pub fn paste_keys_up() -> Result<(), &'static str> {
    Err(UNAVAILABLE)
}
#[allow(dead_code)]
pub fn read_clipboard(
    excluded: impl Fn(&str) -> bool,
) -> Result<Option<super::ClipboardCapture>, &'static str> {
    read_clipboard_filtered(excluded, |_| false)
}
pub fn read_clipboard_filtered(
    _: impl Fn(&str) -> bool,
    _: impl Fn(&str) -> bool,
) -> Result<Option<super::ClipboardCapture>, &'static str> {
    Err(UNAVAILABLE)
}
pub(crate) fn pick_excluded_app(
    _: &tauri::Window,
    _: impl FnOnce(Result<Option<crate::clipboard_preferences::ExcludedApp>, String>) + 'static,
) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}
pub fn write_clipboard(_: &super::ClipboardPayload) -> Result<i64, (&'static str, Option<i64>)> {
    Err((UNAVAILABLE, None))
}

pub fn prepare_image_paste(payload: super::ClipboardPayload) -> super::ClipboardPayload {
    payload
}

pub fn clipboard_image_preview(_: &[u8], _: u32) -> Result<Vec<u8>, &'static str> {
    Err(UNAVAILABLE)
}

pub(crate) fn application_presentation(_: &str) -> Option<(Option<String>, Option<String>)> {
    None
}

pub fn file_icon_data_url(_: &str) -> Option<String> {
    None
}

pub fn open_clipboard_target(_: nami::clipboard::open_target::OpenTarget) -> Result<(), String> {
    Err("platform_unavailable".into())
}

pub(crate) fn pick_image_save_path(
    _: &tauri::Window,
    _: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}

pub(crate) fn write_image_export(_: &std::path::Path, _: Vec<u8>) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}

pub(crate) fn pick_backup_save_path(
    _: &tauri::Window,
    _: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}

pub(crate) fn pick_backup_open_path(
    _: &tauri::Window,
    _: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}

pub fn capture_window_anchor(
    _: nami::model::WindowPositionMode,
) -> Option<nami::model::PlacementAnchor> {
    None
}

pub(crate) fn start_clipboard_drag(
    _: &tauri::Webview,
    _: super::DragItem,
    _: impl FnOnce() + Send + 'static,
    _: impl FnOnce(Result<(), String>) + Send + 'static,
) -> Result<(), String> {
    Err(UNAVAILABLE.into())
}
