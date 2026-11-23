#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardPayload {
    Text(String),
    RichText {
        text: String,
        formatting: nami::clipboard::ClipboardTextFormatting,
    },
    Image {
        bytes: Vec<u8>,
        extension: String,
    },
    Files(Vec<String>),
    FilesImage {
        paths: Vec<String>,
        bytes: Vec<u8>,
        extension: String,
        image_fallback: bool,
    },
}

pub struct ClipboardCapture {
    pub version: i64,
    pub payload: ClipboardPayload,
    pub source_app: Option<serde_json::Value>,
}

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod clipboard;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub use clipboard::*;

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod macos;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub use macos::*;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod selection_copy;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub use selection_copy::*;
mod image_metadata;
pub use image_metadata::image_dimensions;
mod ocr;
pub use ocr::recognize_image_text;

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
mod unsupported;
#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub use unsupported::*;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub platform: &'static str,
    // Native split-pane host; records content is shared React.
    pub native_history: bool,
    pub native_chrome: bool,
    pub native_material: bool,
    pub selection: bool,
    pub image_ocr: bool,
    pub clipboard_write: bool,
    pub auto_paste: bool,
    pub clipboard_open_target: bool,
    pub clipboard_image_save: bool,
    pub accessibility_permission: bool,
    pub primary_modifier: &'static str,
}

pub fn capabilities() -> Capabilities {
    let macos = cfg!(target_os = "macos");
    let native = macos && !cfg!(feature = "portable-ui");
    Capabilities {
        platform: if macos {
            "macos"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "linux") {
            "linux"
        } else {
            "other"
        },
        native_history: native,
        native_chrome: native,
        native_material: native,
        selection: native,
        image_ocr: native,
        clipboard_write: native,
        auto_paste: native,
        clipboard_open_target: native,
        clipboard_image_save: native,
        accessibility_permission: native,
        primary_modifier: if macos { "meta" } else { "control" },
    }
}
