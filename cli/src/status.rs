//! `tokache status`: fetch (with cache + transparent token refresh) and render.

use std::io::IsTerminal;

use anyhow::{Context, Result};
use chrono::Utc;
use tokache_core::accounts::Accounts;
use tokache_core::cache::{Cache, DEFAULT_TTL};
use tokache_core::credentials::CredentialBlob;
use tokache_core::keychain::{current_user, Keychain, CLAUDE_SERVICE};
use tokache_core::usage::Usage;
use tokache_core::{net, now_ms};

use crate::render;

pub fn run(keychain: &dyn Keychain, json: bool, all: bool) -> Result<()> {
    let data_dir = tokache_core::data_dir()?;
    let cache = Cache::new(&data_dir, DEFAULT_TTL);
    if all {
        run_all(keychain, &cache, &data_dir, json)
    } else {
        let body = current_usage(keychain, &cache)?;
        if json {
            println!("{body}");
        } else {
            print_gauges(&Usage::parse(&body)?, "");
        }
        Ok(())
    }
}

/// Usage body for the live login, refreshing the keychain item if expired.
/// A fresh cache hit never touches the keychain (no consent prompt).
fn current_usage(keychain: &dyn Keychain, cache: &Cache) -> Result<String> {
    if let Some(body) = cache.get("current") {
        return Ok(body);
    }
    let (blob, _) = live_blob(keychain)?;
    let blob = ensure_fresh(blob, |b| {
        let user = current_user()?;
        keychain.write(CLAUDE_SERVICE, &user, &b.to_json()?)
    })?;
    fetch_cached(cache, "current", &blob.oauth.access_token)
}

fn run_all(
    keychain: &dyn Keychain,
    cache: &Cache,
    data_dir: &std::path::Path,
    json: bool,
) -> Result<()> {
    let accounts = Accounts::new(keychain, data_dir);
    let mut out = serde_json::Map::new();

    // The live login first, then each named backup.
    let live_subscription = live_blob(keychain)
        .ok()
        .and_then(|(b, _)| b.oauth.subscription_type.clone());
    report(
        json,
        &mut out,
        "current",
        live_subscription.as_deref(),
        current_usage(keychain, cache),
    );

    for meta in accounts.list().context("reading the account index")? {
        let result = (|| {
            if let Some(body) = cache.get(&meta.name) {
                return Ok(body);
            }
            let blob = accounts.read_blob(&meta.name)?;
            // Refresh against the *backup* item; rotation must be persisted
            // or the stored refresh token dies.
            let blob = ensure_fresh(blob, |b| accounts.write_blob(&meta.name, b))?;
            fetch_cached(cache, &meta.name, &blob.oauth.access_token)
        })();
        report(
            json,
            &mut out,
            &meta.name,
            meta.subscription_type.as_deref(),
            result,
        );
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Object(out))?
        );
    }
    Ok(())
}

/// Render (or collect, for --json) one account's outcome. Errors mark the
/// account stale rather than aborting the sweep.
fn report(
    json: bool,
    out: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    subscription: Option<&str>,
    result: Result<String>,
) {
    match result {
        Ok(body) => {
            if json {
                let value =
                    serde_json::from_str(&body).unwrap_or_else(|_| serde_json::Value::String(body));
                out.insert(name.to_string(), value);
            } else {
                match subscription {
                    Some(sub) => println!("{name} ({sub})"),
                    None => println!("{name}"),
                }
                match Usage::parse(&body) {
                    Ok(usage) => print_gauges(&usage, "  "),
                    Err(e) => println!("  unreadable response: {e}"),
                }
            }
        }
        Err(e) => {
            if json {
                out.insert(
                    name.to_string(),
                    serde_json::json!({ "stale": true, "error": e.to_string() }),
                );
            } else {
                println!("{name}: stale — {e}");
                println!("  (re-capture with `tokache accounts remove {name} && tokache accounts add {name}`)");
            }
        }
    }
}

fn print_gauges(usage: &Usage, indent: &str) {
    let color = std::io::stdout().is_terminal();
    let lines = render::gauges(usage, Utc::now(), color);
    if lines.is_empty() {
        println!("{indent}no rate-limit windows reported");
    }
    for line in lines {
        println!("{indent}{line}");
    }
}

/// Read the live Claude Code credentials. Returns the blob and the user name.
pub fn live_blob(keychain: &dyn Keychain) -> Result<(CredentialBlob, String)> {
    let user = current_user()?;
    let raw = keychain
        .read(CLAUDE_SERVICE, &user)?
        .ok_or(tokache_core::Error::NoCredentials)?;
    Ok((CredentialBlob::parse(&raw)?, user))
}

/// Refresh `blob` if its access token is expired, persisting via `write_back`.
fn ensure_fresh(
    blob: CredentialBlob,
    write_back: impl FnOnce(&CredentialBlob) -> tokache_core::Result<()>,
) -> Result<CredentialBlob> {
    if !blob.oauth.is_expired_at(now_ms()) {
        return Ok(blob);
    }
    let fresh = net::refresh(&blob.oauth, now_ms()).context("refreshing expired access token")?;
    let updated = blob.with_oauth(fresh)?;
    write_back(&updated).context("writing refreshed credentials back")?;
    Ok(updated)
}

fn fetch_cached(cache: &Cache, key: &str, access_token: &str) -> Result<String> {
    if let Some(body) = cache.get(key) {
        return Ok(body);
    }
    let body = net::fetch_usage(access_token)?;
    cache.put(key, &body)?;
    Ok(body)
}
