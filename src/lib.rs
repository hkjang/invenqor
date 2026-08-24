pub mod collectors;
pub mod config;
pub mod diagnose;
pub mod health;
pub mod identity;
pub mod logfile;
pub mod model;
pub mod platform;
pub mod scheduler;
pub mod service_identity;
pub mod smbios;
pub mod storage;
pub mod transport;
pub mod updater;
pub mod windows_inventory;

#[cfg(windows)]
pub mod windows_service;
#[cfg(windows)]
pub mod windows_sys;
