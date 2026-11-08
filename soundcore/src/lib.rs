//! Noise control and battery for Soundcore headphones.

pub mod models;
pub mod protocol;
pub mod settings;

#[cfg(target_arch = "wasm32")]
mod plugin;
