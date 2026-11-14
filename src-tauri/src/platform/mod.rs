#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;
#[cfg(target_os = "macos")]
mod selection_copy;
#[cfg(target_os = "macos")]
pub use selection_copy::*;

#[cfg(not(target_os = "macos"))]
compile_error!("Nami P0 currently supports macOS only");
