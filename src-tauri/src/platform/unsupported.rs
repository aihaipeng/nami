// Ordinary translation/history windows work before OS integrations are implemented.
const UNAVAILABLE: &str = "platform_unavailable";

#[derive(Clone, Copy, Debug)]
pub struct Target;

pub fn capture_target() -> Option<Target> {
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
pub fn request_accessibility() -> bool {
    false
}
pub fn selected_text(_: Target) -> Result<String, &'static str> {
    Err(UNAVAILABLE)
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
pub fn read_clipboard() -> Result<Option<super::ClipboardCapture>, &'static str> {
    Err(UNAVAILABLE)
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

pub fn file_icon_data_url(_: &str) -> Option<String> {
    None
}
