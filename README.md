# tokache 🦝

Token mapache: tracks usage limits across AI subscription accounts. Eventually a
macOS menubar app with account auto-switching; today a small CLI.

**Status: M0 — Claude account tracking CLI.** macOS only.

```
$ tokache status
5h        [██████████░░░░░░░░░░░░░░░░░░░░]   34%  resets in 2h 14m (at 4:00 PM)
7d        [██████████████████░░░░░░░░░░░░]   61%  resets in 3d 12h (Fri 2:00 AM)
```

- `tokache status` — usage gauges for the current Claude Code login
  (`--json` for the raw response, `--all` to include named accounts)
- `tokache accounts add <name>` — capture the current login as a named backup
- `tokache accounts list` / `remove <name>`

Credentials never touch disk: backups live as individual items in your login
keychain; only names and timestamps go in
`~/Library/Application Support/tokache/`. Reading Claude Code's keychain item
triggers a macOS consent prompt — that's expected.

## Install from source

```sh
git clone https://github.com/juliankmazo/tokache
cd tokache
cargo install --path cli
```

## Terms-of-service posture

Anthropic's policy restricts subscription OAuth use outside Claude Code and
Claude.ai. tokache does no inference with your tokens — it reads the same
usage numbers `/usage` shows, polls at low frequency with hard caching, and
exists to manage legitimately separate accounts (e.g. personal + work), not to
evade limits. This mirrors what public tools like CCSeva and claude-swap do,
but it relies on an undocumented endpoint: use at your own judgment.

## License

[FSL-1.1-MIT](LICENSE) (Fair Source). Free to use, modify, and share for any
purpose except offering tokache as a competing commercial product or service —
that requires a license from me. Each version becomes plain MIT two years
after release.
