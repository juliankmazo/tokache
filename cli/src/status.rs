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
    let accounts = Accounts::new(keychain, &data_dir);
    if all {
        run_all(keychain, &accounts, &cache, json)
    } else {
        let body = current_usage(keychain, &accounts, &cache).map_err(|e| {
            if is_401(&e) {
                e.context(
                    "the usage endpoint rejected the token — re-authenticate with `claude /login`",
                )
            } else {
                e
            }
        })?;
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
fn current_usage(keychain: &dyn Keychain, accounts: &Accounts, cache: &Cache) -> Result<String> {
    if let Some(body) = cache.get("current") {
        return Ok(body);
    }
    let (blob, _) = live_blob(keychain)?;
    let blob = ensure_fresh(keychain, accounts, blob, None)?;
    fetch_and_cache(cache, "current", &blob.oauth.access_token)
}

fn run_all(keychain: &dyn Keychain, accounts: &Accounts, cache: &Cache, json: bool) -> Result<()> {
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
        current_usage(keychain, accounts, cache),
    );

    for meta in accounts.list().context("reading the account index")? {
        let result = (|| {
            if let Some(body) = cache.get(&meta.name) {
                return Ok(body);
            }
            let blob = accounts.read_blob(&meta.name)?;
            let blob = ensure_fresh(keychain, accounts, blob, Some(&meta.name))?;
            fetch_and_cache(cache, &meta.name, &blob.oauth.access_token)
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
                if name == "current" {
                    if is_401(&e) {
                        println!("  (re-authenticate with `claude /login`)");
                    }
                } else {
                    println!("  (re-capture with `tokache accounts remove {name} && tokache accounts add {name}`)");
                }
            }
        }
    }
}

/// Was this a 401 from the usage endpoint (token no longer accepted)?
fn is_401(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<tokache_core::Error>(),
        Some(tokache_core::Error::Http { status: 401, .. })
    )
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

/// Refresh `blob` if its access token is expired, persisting to the copy it
/// came from (`backup_name`, or the live Claude Code item for `None`) and
/// then syncing every other stored copy that held the same rotated refresh
/// token — right after `accounts add`, live and backup share one, and a
/// rotation persisted to only one copy would strand the other.
fn ensure_fresh(
    keychain: &dyn Keychain,
    accounts: &Accounts,
    blob: CredentialBlob,
    backup_name: Option<&str>,
) -> Result<CredentialBlob> {
    if !blob.oauth.is_expired_at(now_ms()) {
        return Ok(blob);
    }
    let old_refresh = blob.oauth.refresh_token.clone();
    let fresh = net::refresh(&blob.oauth, now_ms()).context("refreshing expired access token")?;
    let updated = blob.with_oauth(fresh.clone())?;
    let user = current_user()?;
    match backup_name {
        Some(name) => accounts.write_blob(name, &updated),
        None => keychain.write(CLAUDE_SERVICE, &user, &updated.to_json()?),
    }
    .context("writing refreshed credentials back")?;
    accounts
        .sync_rotated(&user, &old_refresh, &fresh, backup_name)
        .context("syncing rotated credentials to other stored copies")?;
    Ok(updated)
}

fn fetch_and_cache(cache: &Cache, key: &str, access_token: &str) -> Result<String> {
    let body = net::fetch_usage(access_token)?;
    cache.put(key, &body)?;
    Ok(body)
}
