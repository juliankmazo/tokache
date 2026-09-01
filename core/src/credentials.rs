//! Parsing and careful re-serialization of Claude Code's credential blob.
//!
//! The blob is one JSON object: `{ "claudeAiOauth": {...}, "mcpOAuth": {...}, ... }`.
//! We only ever *understand* `claudeAiOauth`; everything else (notably
//! `mcpOAuth`, which holds MCP server logins) is kept as opaque JSON and
//! written back untouched.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

/// Treat a token as expired this long before its actual deadline.
pub const EXPIRY_SKEW_MS: i64 = 60_000;

const OAUTH_KEY: &str = "claudeAiOauth";

/// The `claudeAiOauth` section. Unknown fields are round-tripped via `extra`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeOauth {
    pub access_token: String,
    pub refresh_token: String,
    /// Epoch milliseconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_tier: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl ClaudeOauth {
    /// Expired (or about to be) at `now_ms`?
    pub fn is_expired_at(&self, now_ms: i64) -> bool {
        now_ms + EXPIRY_SKEW_MS >= self.expires_at
    }
}

/// The whole keychain blob: parsed `claudeAiOauth` plus the raw document.
#[derive(Debug, Clone)]
pub struct CredentialBlob {
    raw: Value,
    pub oauth: ClaudeOauth,
}

impl CredentialBlob {
    pub fn parse(json: &str) -> Result<Self> {
        let raw: Value =
            serde_json::from_str(json).map_err(|e| Error::BadBlob(format!("not JSON: {e}")))?;
        let oauth_val = raw
            .get(OAUTH_KEY)
            .ok_or_else(|| Error::BadBlob(format!("missing {OAUTH_KEY}")))?;
        let oauth: ClaudeOauth = serde_json::from_value(oauth_val.clone())
            .map_err(|e| Error::BadBlob(format!("bad {OAUTH_KEY}: {e}")))?;
        Ok(Self { raw, oauth })
    }

    /// A copy of this blob with only `claudeAiOauth` replaced. Every other
    /// key (`mcpOAuth`, anything unknown) is preserved as-is.
    pub fn with_oauth(&self, oauth: ClaudeOauth) -> Result<Self> {
        let mut raw = self.raw.clone();
        let obj = raw
            .as_object_mut()
            .ok_or_else(|| Error::BadBlob("blob is not a JSON object".into()))?;
        obj.insert(OAUTH_KEY.to_string(), serde_json::to_value(&oauth)?);
        Ok(Self { raw, oauth })
    }

    /// Serialize for writing back to the keychain.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Layout verified in PLAN.md; token values are fakes.
    const FIXTURE: &str = concat!(
        r#"{"claudeAiOauth":{"accessToken":"fake-access","refreshToken":"fake-refresh","#,
        r#""expiresAt":1756672496000,"scopes":["user:inference","user:profile"],"#,
        r#""subscriptionType":"max","rateLimitTier":"default_max_20x","#,
        r#""futureField":true},"#,
        r#""mcpOAuth":{"someServer":{"accessToken":"fake-mcp","opaque":[1,2,3]}},"#,
        r#""unknownTopLevel":"keep me"}"#
    );

    #[test]
    fn parses_oauth_section() {
        let blob = CredentialBlob::parse(FIXTURE).unwrap();
        assert_eq!(blob.oauth.access_token, "fake-access");
        assert_eq!(blob.oauth.refresh_token, "fake-refresh");
        assert_eq!(blob.oauth.expires_at, 1756672496000);
        assert_eq!(blob.oauth.scopes, ["user:inference", "user:profile"]);
        assert_eq!(blob.oauth.subscription_type.as_deref(), Some("max"));
        assert_eq!(
            blob.oauth.rate_limit_tier.as_deref(),
            Some("default_max_20x")
        );
        assert_eq!(blob.oauth.extra["futureField"], serde_json::json!(true));
    }

    #[test]
    fn with_oauth_replaces_only_claude_ai_oauth() {
        let blob = CredentialBlob::parse(FIXTURE).unwrap();
        let mut fresh = blob.oauth.clone();
        fresh.access_token = "new-access".into();
        fresh.refresh_token = "new-refresh".into();
        fresh.expires_at = 1756700000000;

        let updated = blob.with_oauth(fresh).unwrap();
        let json = updated.to_json().unwrap();
        let value: Value = serde_json::from_str(&json).unwrap();

        // mcpOAuth and unknown top-level keys byte-identical to the original.
        let original: Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(value["mcpOAuth"], original["mcpOAuth"]);
        assert_eq!(value["unknownTopLevel"], original["unknownTopLevel"]);
        // Top-level key order preserved.
        let keys: Vec<_> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["claudeAiOauth", "mcpOAuth", "unknownTopLevel"]);
        // New tokens in place, unknown oauth field kept.
        assert_eq!(value["claudeAiOauth"]["accessToken"], "new-access");
        assert_eq!(value["claudeAiOauth"]["refreshToken"], "new-refresh");
        assert_eq!(value["claudeAiOauth"]["expiresAt"], 1756700000000i64);
        assert_eq!(
            value["claudeAiOauth"]["futureField"],
            serde_json::json!(true)
        );
    }

    #[test]
    fn expiry_includes_skew() {
        let blob = CredentialBlob::parse(FIXTURE).unwrap();
        let at = blob.oauth.expires_at;
        assert!(!blob.oauth.is_expired_at(at - EXPIRY_SKEW_MS - 1));
        assert!(blob.oauth.is_expired_at(at - EXPIRY_SKEW_MS));
        assert!(blob.oauth.is_expired_at(at + 1));
    }

    #[test]
    fn missing_oauth_is_bad_blob() {
        assert!(matches!(
            CredentialBlob::parse(r#"{"mcpOAuth":{}}"#),
            Err(crate::Error::BadBlob(_))
        ));
        assert!(matches!(
            CredentialBlob::parse("not json"),
            Err(crate::Error::BadBlob(_))
        ));
    }
}
