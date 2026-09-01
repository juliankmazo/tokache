//! Thin keychain seam. The trait keeps everything above it testable with an
//! in-memory implementation; [`LoginKeychain`] is the real thing.
//!
//! Secrets travel only through process memory and Security.framework —
//! never argv, never logs.

use crate::{Error, Result};

/// Claude Code's own credential item (account = current user).
pub const CLAUDE_SERVICE: &str = "Claude Code-credentials";
/// tokache's backup items (account = the backup's name).
pub const TOKACHE_SERVICE: &str = "tokache";

pub trait Keychain {
    /// `Ok(None)` when the item does not exist.
    fn read(&self, service: &str, account: &str) -> Result<Option<String>>;
    /// Create or replace the item.
    fn write(&self, service: &str, account: &str, secret: &str) -> Result<()>;
    fn delete(&self, service: &str, account: &str) -> Result<()>;
}

/// The macOS user whose login Claude Code stores credentials under.
pub fn current_user() -> Result<String> {
    std::env::var("USER").map_err(|_| Error::Keychain("USER is not set".into()))
}

#[cfg(target_os = "macos")]
pub use macos::LoginKeychain;

#[cfg(target_os = "macos")]
mod macos {
    use super::{Error, Keychain, Result};
    use security_framework::passwords;

    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    /// The real login keychain, via Security.framework.
    pub struct LoginKeychain;

    impl Keychain for LoginKeychain {
        fn read(&self, service: &str, account: &str) -> Result<Option<String>> {
            match passwords::get_generic_password(service, account) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| Error::Keychain("item data is not UTF-8".into())),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(Error::Keychain(e.to_string())),
            }
        }

        fn write(&self, service: &str, account: &str, secret: &str) -> Result<()> {
            passwords::set_generic_password(service, account, secret.as_bytes())
                .map_err(|e| Error::Keychain(e.to_string()))
        }

        fn delete(&self, service: &str, account: &str) -> Result<()> {
            match passwords::delete_generic_password(service, account) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
                Err(e) => Err(Error::Keychain(e.to_string())),
            }
        }
    }
}
