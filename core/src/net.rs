//! The two Anthropic endpoints M0 talks to. Blocking, no retries — callers
//! surface errors and let the (cached) next invocation try again.

use serde::Deserialize;

use crate::credentials::ClaudeOauth;
use crate::{Error, Result};

pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
/// Claude Code's public OAuth client id.
pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Without a claude-code UA the usage endpoint applies an aggressive 429 bucket.
pub const USER_AGENT: &str = "claude-code/2.12.0";
pub const OAUTH_BETA: &str = "oauth-2025-04-20";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
}

fn map_err(endpoint: &'static str) -> impl Fn(ureq::Error) -> Error {
    move |e| match e {
        ureq::Error::Status(status, _) => Error::Http { endpoint, status },
        // Transport errors never carry headers or bodies, so no token leak.
        ureq::Error::Transport(t) => Error::Network {
            endpoint,
            detail: t.to_string(),
        },
    }
}

/// Fetch the raw usage JSON for `access_token`.
pub fn fetch_usage(access_token: &str) -> Result<String> {
    let resp = agent()
        .get(USAGE_URL)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("anthropic-beta", OAUTH_BETA)
        .call()
        .map_err(map_err("usage"))?;
    Ok(resp.into_string()?)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Seconds until expiry.
    expires_in: i64,
}

/// Exchange the refresh token for fresh credentials. The refresh token may
/// rotate; the returned [`ClaudeOauth`] must be written back wherever the
/// input came from or the stored copy dies.
pub fn refresh(oauth: &ClaudeOauth, now_ms: i64) -> Result<ClaudeOauth> {
    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "refresh_token": oauth.refresh_token,
        "client_id": CLIENT_ID,
    });
    let resp = agent()
        .post(TOKEN_URL)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(map_err("token"))?;
    let token: TokenResponse = serde_json::from_str(&resp.into_string()?)?;

    let mut fresh = oauth.clone();
    fresh.access_token = token.access_token;
    if let Some(rt) = token.refresh_token {
        fresh.refresh_token = rt;
    }
    fresh.expires_at = now_ms + token.expires_in * 1000;
    Ok(fresh)
}
