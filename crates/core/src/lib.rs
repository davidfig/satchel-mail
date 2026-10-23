//! Satchel Mail core: local store, sync engine, op queue, proposals, search
//! and the AI mirror. Hosts (desktop app, CLI, future server) wrap [`Engine`].

pub mod blobs;
pub mod calendar;
mod commands;
pub mod engine;
pub mod error;
pub mod mirror;
mod ops;
pub mod provider;
pub mod search;
pub mod secrets;
pub mod store;

pub use engine::{Caller, Engine, EngineOptions, Mode, default_data_dir};
