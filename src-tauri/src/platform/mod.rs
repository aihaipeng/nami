#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod macos;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub use macos::*;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod selection_copy;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub use selection_copy::*;

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
mod unsupported;
#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub use unsupported::*;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub platform: &'static str,
    pub native_history: bool,
    pub native_chrome: bool,
    pub native_material: bool,
    pub selection: bool,
    pub clipboard_write: bool,
    pub auto_paste: bool,
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
        clipboard_write: native,
        auto_paste: native,
        accessibility_permission: native,
        primary_modifier: if macos { "meta" } else { "control" },
    }
}
