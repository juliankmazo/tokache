//! Named account backups.
//!
//! The secret blob for each backup lives in its own keychain item
//! (service [`TOKACHE_SERVICE`], account = the backup name). Only non-secret
//! metadata goes in `accounts.json` under the data dir.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::credentials::{ClaudeOauth, CredentialBlob};
use crate::keychain::{Keychain, CLAUDE_SERVICE, TOKACHE_SERVICE};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountMeta {
    pub name: String,
    /// RFC 3339, when the backup was captured.
    pub added_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    accounts: Vec<AccountMeta>,
}

/// The account registry: JSON index on disk + one keychain item per backup.
pub struct Accounts<'k> {
    keychain: &'k dyn Keychain,
    index_path: PathBuf,
}

impl<'k> Accounts<'k> {
    pub fn new(keychain: &'k dyn Keychain, data_dir: &Path) -> Self {
        Self {
            keychain,
            index_path: data_dir.join("accounts.json"),
        }
    }

    pub fn list(&self) -> Result<Vec<AccountMeta>> {
        Ok(self.load()?.accounts)
    }

    /// Capture `blob` (the currently logged-in credentials) as backup `name`.
    pub fn add(&self, name: &str, blob: &CredentialBlob, added_at: &str) -> Result<AccountMeta> {
        validate_name(name)?;
        let secret = blob.to_json()?;
        let mut index = self.load()?;
        if index.accounts.iter().any(|a| a.name == name) {
            return Err(Error::AccountExists(name.to_string()));
        }
        let meta = AccountMeta {
            name: name.to_string(),
            added_at: added_at.to_string(),
            subscription_type: blob.oauth.subscription_type.clone(),
        };
        // Index first, keychain second: if the keychain write fails, roll the
        // index back so nothing ever lists an account whose secret was never
        // stored, and a failed save never orphans an invisible keychain item.
        index.accounts.push(meta.clone());
        self.save(&index)?;
        if let Err(e) = self.keychain.write(TOKACHE_SERVICE, name, &secret) {
            index.accounts.pop();
            let _ = self.save(&index);
            return Err(e);
        }
        Ok(meta)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let mut index = self.load()?;
        let before = index.accounts.len();
        index.accounts.retain(|a| a.name != name);
        if index.accounts.len() == before {
            return Err(Error::AccountNotFound(name.to_string()));
        }
        // Index first: if the keychain delete then fails, no phantom listing
        // blocks a re-add, and delete tolerates not-found so a retry converges.
        self.save(&index)?;
        self.keychain.delete(TOKACHE_SERVICE, name)
    }

    /// Read a backup's credential blob from the keychain.
    pub fn read_blob(&self, name: &str) -> Result<CredentialBlob> {
        match self.keychain.read(TOKACHE_SERVICE, name)? {
            Some(json) => CredentialBlob::parse(&json),
            None => Err(Error::AccountNotFound(name.to_string())),
        }
    }

    /// Overwrite a backup's blob (e.g. after a token refresh rotated it).
    pub fn write_blob(&self, name: &str, blob: &CredentialBlob) -> Result<()> {
        self.keychain.write(TOKACHE_SERVICE, name, &blob.to_json()?)
    }

    /// After a refresh rotated credentials, bring every *other* stored copy
    /// that still holds the pre-refresh refresh token up to date: the live
    /// Claude Code item (account `live_user`) and all named backups. Right
    /// after `accounts add`, live and backup share one refresh token, so a
    /// rotation persisted to only one copy would strand the other with a
    /// dead token (→ forced `/login`). Only the rotating fields (access
    /// token, refresh token, expiry) are copied; each target keeps its own
    /// scopes, subscription type, and unknown fields.
    ///
    /// Best-effort: the refresh already succeeded, so one unreachable copy
    /// must not stop the others from being synced. Each failure is returned
    /// as a warning for the caller to surface — never as an error.
    ///
    /// `already_updated` is the copy the refresh was written to: a backup
    /// name, or `None` for the live item.
    #[must_use]
    pub fn sync_rotated(
        &self,
        live_user: &str,
        old_refresh_token: &str,
        fresh: &ClaudeOauth,
        already_updated: Option<&str>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        if already_updated.is_some() {
            if let Err(e) = self.sync_live(live_user, old_refresh_token, fresh) {
                warnings.push(format!("live Claude Code item: {e}"));
            }
        }
        match self.list() {
            Ok(metas) => {
                for meta in metas {
                    if Some(meta.name.as_str()) == already_updated {
                        continue;
                    }
                    if let Err(e) = self.sync_backup(&meta.name, old_refresh_token, fresh) {
                        warnings.push(format!("backup '{}': {e}", meta.name));
                    }
                }
            }
            Err(e) => warnings.push(format!("account index unreadable: {e}")),
        }
        warnings
    }

    fn sync_live(&self, live_user: &str, old: &str, fresh: &ClaudeOauth) -> Result<()> {
        if let Some(raw) = self.keychain.read(CLAUDE_SERVICE, live_user)? {
            let blob = CredentialBlob::parse(&raw)?;
            if blob.oauth.refresh_token == old {
                let updated = blob.with_oauth(rotated(&blob.oauth, fresh))?;
                self.keychain
                    .write(CLAUDE_SERVICE, live_user, &updated.to_json()?)?;
            }
        }
        Ok(())
    }

    fn sync_backup(&self, name: &str, old: &str, fresh: &ClaudeOauth) -> Result<()> {
        let blob = self.read_blob(name)?;
        if blob.oauth.refresh_token == old {
            self.write_blob(name, &blob.with_oauth(rotated(&blob.oauth, fresh))?)?;
        }
        Ok(())
    }

    fn load(&self) -> Result<Index> {
        match std::fs::read_to_string(&self.index_path) {
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, index: &Index) -> Result<()> {
        if let Some(dir) = self.index_path.parent() {
            crate::create_private_dir(dir)?;
        }
        let json = serde_json::to_string_pretty(index)?;
        std::fs::write(&self.index_path, json)?;
        Ok(())
    }
}

/// The target's own oauth with only the rotating fields taken from `fresh`.
fn rotated(own: &ClaudeOauth, fresh: &ClaudeOauth) -> ClaudeOauth {
    let mut oauth = own.clone();
    oauth.access_token = fresh.access_token.clone();
    oauth.refresh_token = fresh.refresh_token.clone();
    oauth.expires_at = fresh.expires_at;
    oauth
}

fn validate_name(name: &str) -> Result<()> {
    // "current" is how `status --all` and the cache identify the live login.
    if name.eq_ignore_ascii_case("current") {
        return Err(Error::ReservedName(name.to_string()));
    }
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(Error::BadAccountName(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct MemKeychain(RefCell<HashMap<(String, String), String>>);

    impl MemKeychain {
        fn new() -> Self {
            Self(RefCell::new(HashMap::new()))
        }
    }

    impl Keychain for MemKeychain {
        fn read(&self, service: &str, account: &str) -> Result<Option<String>> {
            Ok(self
                .0
                .borrow()
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn write(&self, service: &str, account: &str, secret: &str) -> Result<()> {
            self.0
                .borrow_mut()
                .insert((service.into(), account.into()), secret.into());
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> Result<()> {
            self.0
                .borrow_mut()
                .remove(&(service.into(), account.into()));
            Ok(())
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tokache-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn fixture_blob() -> CredentialBlob {
        CredentialBlob::parse(
            r#"{"claudeAiOauth":{"accessToken":"at-x","refreshToken":"rt-x",
                "expiresAt":1750000000000,"scopes":["user:inference"],
                "subscriptionType":"max"},"mcpOAuth":{"srv":{"t":"opaque"}}}"#,
        )
        .unwrap()
    }

    #[test]
    fn add_list_remove_roundtrip() {
        let kc = MemKeychain::new();
        let dir = temp_dir("roundtrip");
        let accounts = Accounts::new(&kc, &dir);

        accounts
            .add("work", &fixture_blob(), "2026-08-31T12:00:00Z")
            .unwrap();
        let listed = accounts.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "work");
        assert_eq!(listed[0].subscription_type.as_deref(), Some("max"));

        // Index file must not contain token material.
        let index = std::fs::read_to_string(dir.join("accounts.json")).unwrap();
        assert!(!index.contains("at-x") && !index.contains("rt-x"));

        // Blob (with mcpOAuth) survives in the keychain.
        let blob = accounts.read_blob("work").unwrap();
        assert!(blob.to_json().unwrap().contains("mcpOAuth"));

        accounts.remove("work").unwrap();
        assert!(accounts.list().unwrap().is_empty());
        assert!(matches!(
            accounts.read_blob("work"),
            Err(Error::AccountNotFound(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_and_missing_are_errors() {
        let kc = MemKeychain::new();
        let dir = temp_dir("dups");
        let accounts = Accounts::new(&kc, &dir);
        accounts
            .add("a", &fixture_blob(), "2026-08-31T12:00:00Z")
            .unwrap();
        assert!(matches!(
            accounts.add("a", &fixture_blob(), "2026-08-31T12:00:00Z"),
            Err(Error::AccountExists(_))
        ));
        assert!(matches!(
            accounts.remove("nope"),
            Err(Error::AccountNotFound(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn blob_with_refresh(rt: &str) -> CredentialBlob {
        CredentialBlob::parse(&format!(
            r#"{{"claudeAiOauth":{{"accessToken":"at-{rt}","refreshToken":"{rt}",
                "expiresAt":1750000000000,"scopes":["user:inference"],
                "subscriptionType":"max"}},"mcpOAuth":{{"srv":{{"t":"opaque"}}}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn sync_rotated_updates_every_copy_sharing_the_old_token() {
        let kc = MemKeychain::new();
        let dir = temp_dir("sync");
        let accounts = Accounts::new(&kc, &dir);

        // Live login and the "shared" backup hold the same refresh token (the
        // state right after `accounts add`); "other" does not. The live copy
        // has its own non-token fields that a sync must not clobber.
        let shared = blob_with_refresh("rt-shared");
        let live_orig = CredentialBlob::parse(
            r#"{"claudeAiOauth":{"accessToken":"at-live","refreshToken":"rt-shared",
                "expiresAt":1750000000000,"scopes":["user:profile"],
                "subscriptionType":"pro","liveMarker":true},
                "mcpOAuth":{"srv":{"t":"opaque"}}}"#,
        )
        .unwrap();
        kc.write(CLAUDE_SERVICE, "julian", &live_orig.to_json().unwrap())
            .unwrap();
        accounts
            .add("shared", &shared, "2026-08-31T12:00:00Z")
            .unwrap();
        accounts
            .add(
                "other",
                &blob_with_refresh("rt-other"),
                "2026-08-31T12:00:00Z",
            )
            .unwrap();

        // A refresh of the "shared" backup rotated the token. The caller
        // writes the refreshed copy itself, then syncs the rest.
        let mut fresh = shared.oauth.clone();
        fresh.access_token = "at-new".into();
        fresh.refresh_token = "rt-new".into();
        fresh.expires_at = 1750009999000;
        accounts
            .write_blob("shared", &shared.with_oauth(fresh.clone()).unwrap())
            .unwrap();
        let warnings = accounts.sync_rotated("julian", "rt-shared", &fresh, Some("shared"));
        assert!(warnings.is_empty(), "{warnings:?}");

        // The live item caught up on the rotating fields only — its own
        // scopes, subscriptionType, extra fields, and mcpOAuth are intact.
        let live = kc.read(CLAUDE_SERVICE, "julian").unwrap().unwrap();
        let live = CredentialBlob::parse(&live).unwrap();
        assert_eq!(live.oauth.refresh_token, "rt-new");
        assert_eq!(live.oauth.access_token, "at-new");
        assert_eq!(live.oauth.expires_at, 1750009999000);
        assert_eq!(live.oauth.subscription_type.as_deref(), Some("pro"));
        assert_eq!(live.oauth.scopes, ["user:profile"]);
        assert_eq!(live.oauth.extra["liveMarker"], serde_json::json!(true));
        assert!(live.to_json().unwrap().contains("mcpOAuth"));

        // The unrelated backup is untouched.
        let other = accounts.read_blob("other").unwrap();
        assert_eq!(other.oauth.refresh_token, "rt-other");

        // Mirror case: a refresh of the *live* login syncs backups.
        let mut fresh2 = fresh.clone();
        fresh2.refresh_token = "rt-newer".into();
        let warnings = accounts.sync_rotated("julian", "rt-new", &fresh2, None);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            accounts.read_blob("shared").unwrap().oauth.refresh_token,
            "rt-newer"
        );
        assert_eq!(
            accounts.read_blob("other").unwrap().oauth.refresh_token,
            "rt-other"
        );
        // `None` means the live item was already written by the caller.
        let live = kc.read(CLAUDE_SERVICE, "julian").unwrap().unwrap();
        assert_eq!(
            CredentialBlob::parse(&live).unwrap().oauth.refresh_token,
            "rt-new"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_names_rejected() {
        let kc = MemKeychain::new();
        let dir = temp_dir("names");
        let accounts = Accounts::new(&kc, &dir);
        for bad in ["", "has space", "sl/ash", &"x".repeat(65)] {
            assert!(matches!(
                accounts.add(bad, &fixture_blob(), "2026-08-31T12:00:00Z"),
                Err(Error::BadAccountName(_))
            ));
        }
        // "current" is the live login's identity in status --all and the cache.
        for reserved in ["current", "Current", "CURRENT"] {
            assert!(matches!(
                accounts.add(reserved, &fixture_blob(), "2026-08-31T12:00:00Z"),
                Err(Error::ReservedName(_))
            ));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A keychain whose writes always fail, for the add-rollback path.
    struct WriteFails;
    impl Keychain for WriteFails {
        fn read(&self, _: &str, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn write(&self, _: &str, _: &str, _: &str) -> Result<()> {
            Err(Error::Keychain("simulated write failure".into()))
        }
        fn delete(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn failed_keychain_write_rolls_the_index_back() {
        let kc = WriteFails;
        let dir = temp_dir("rollback");
        let accounts = Accounts::new(&kc, &dir);
        assert!(matches!(
            accounts.add("work", &fixture_blob(), "2026-08-31T12:00:00Z"),
            Err(Error::Keychain(_))
        ));
        // No phantom listing: the failed add left the index empty.
        assert!(accounts.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
