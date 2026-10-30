//! Place lookups for event locations (0050): OpenStreetMap's Photon
//! everywhere, Apple Maps in the Mac app.

#[cfg(target_os = "macos")]
pub mod apple_maps;
pub mod photon;

pub use photon::{PHOTON_URL, Photon};
