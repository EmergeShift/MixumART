//! macOS native surface: MTKView overlay rendering MAP PNG frames with Metal.
//! Input/IME/clipboard stay in the WebView; the overlay is hit-test transparent.

#[cfg(target_os = "macos")]
mod metal_view;
#[cfg(target_os = "macos")]
pub use metal_view::{attach, detach, push_frame};
