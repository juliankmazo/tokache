mod render;
mod status;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use tokache_core::accounts::Accounts;
use tokache_core::keychain::{Keychain, LoginKeychain};

#[derive(Parser)]
#[command(
    name = "tokache",
    version,
    about = "Track Claude usage limits across accounts 🦝"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage named backups of Claude logins
    #[command(subcommand)]
    Accounts(AccountsCmd),
    /// Show usage gauges for the current login
    Status {
        /// Print the raw usage response as JSON
        #[arg(long)]
        json: bool,
        /// Include every named account, not just the current login
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum AccountsCmd {
    /// Capture the currently logged-in Claude credentials under a name
    Add { name: String },
    /// List captured accounts (never shows token material)
    List,
    /// Delete a captured account and its keychain item
    Remove { name: String },
}

fn main() {
    let cli = Cli::parse();
    let keychain = LoginKeychain;
    let result = match cli.command {
        Command::Accounts(cmd) => accounts(&keychain, cmd),
        Command::Status { json, all } => status::run(&keychain, json, all),
    };
    if let Err(e) = result {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn accounts(keychain: &dyn Keychain, cmd: AccountsCmd) -> Result<()> {
    let data_dir = tokache_core::data_dir()?;
    let accounts = Accounts::new(keychain, &data_dir);
    match cmd {
        AccountsCmd::Add { name } => {
            let (blob, _) =
                status::live_blob(keychain).context("capturing the current Claude login")?;
            let added_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let meta = accounts.add(&name, &blob, &added_at)?;
            match meta.subscription_type {
                Some(sub) => println!("Captured current Claude login as '{name}' ({sub})."),
                None => println!("Captured current Claude login as '{name}'."),
            }
            println!("Note: backups go stale when Claude Code rotates its refresh token; re-add now and then.");
            Ok(())
        }
        AccountsCmd::List => {
            let list = accounts.list()?;
            if list.is_empty() {
                println!("No accounts captured yet. Run `tokache accounts add <name>`.");
                return Ok(());
            }
            println!("{:<20} {:<14} ADDED", "NAME", "SUBSCRIPTION");
            for a in list {
                let sub = a.subscription_type.as_deref().unwrap_or("-");
                let added = a.added_at.split('T').next().unwrap_or(&a.added_at);
                println!("{:<20} {:<14} {added}", a.name, sub);
            }
            Ok(())
        }
        AccountsCmd::Remove { name } => {
            accounts.remove(&name)?;
            println!("Removed account '{name}'.");
            Ok(())
        }
    }
}
