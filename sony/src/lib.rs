//! Noise control, battery and settings for Sony headphones.

pub mod protocol;
pub mod settings;
pub mod status;

#[cfg(target_arch = "wasm32")]
mod plugin;
