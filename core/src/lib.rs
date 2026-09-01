//! tokache core: Claude credential parsing, keychain access, and usage polling.
//!
//! Everything network- or keychain-shaped lives behind small seams
//! ([`keychain::Keychain`], the free functions in [`net`]) so the pure logic
//! is testable without touching the real login keychain.

pub mod accounts;
pub mod cache;
pub mod credentials;
pub mod keychain;
pub mod net;
pub mod usage;

mod error;

pub use error::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// `~/Library/Application Support/tokache` — non-secret metadata and caches.
pub fn data_dir() -> Result<std::path::PathBuf> {
    let home = std::env::var("HOME").map_err(|_| Error::NoHome)?;
    Ok(std::path::PathBuf::from(home).join("Library/Application Support/tokache"))
}

/// Current epoch time in milliseconds (the unit `expiresAt` uses).
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
