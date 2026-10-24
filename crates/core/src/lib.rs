//! Satchel Mail core: local store, sync engine, op queue, proposals, search
//! and the AI mirror. Hosts (desktop app, CLI, future server) wrap [`Engine`].

pub mod blobs;
pub mod calendar;
mod commands;
pub mod engine;
pub mod error;
pub mod mirror;
mod ops;
pub mod proposal_file;
pub mod provider;
pub mod search;
pub mod secrets;
mod spam;
pub mod store;

pub use engine::{Caller, Engine, EngineOptions, Mode, default_data_dir};

/// URL-safe random token with `bytes` bytes of OS randomness (UUID v4 draws
/// from the OS RNG, so no separate RNG dependency is needed).
pub fn random_token(bytes: usize) -> String {
    use base64::Engine as _;
    let mut buf = Vec::with_capacity(bytes + 16);
    while buf.len() < bytes {
        buf.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    buf.truncate(bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}
