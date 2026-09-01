# tokache 🦝 — project plan

> Token mapache: tracks usage limits across AI subscription accounts, auto-switches
> accounts when one runs dry, and nags you when quota is about to reset unused.
> macOS menubar app + CLI. Open source.

## Decisions (locked 2026-08-31)

| Decision | Choice | Why |
|---|---|---|
| Stack | **Hybrid: Rust core + CLI, Swift menubar UI** | Learn both languages; Rust has the best CLI ecosystem, Swift is mandatory for iStat-quality UI and WidgetKit. No FFI — the state store is the boundary. |
| Install story | **One artifact: Tokache.app bundles the Rust CLI** (`Contents/Helpers/tokache`), symlinked to PATH (the Docker/VS Code pattern) | Users install one thing; app+CLI always version-matched; one release pipeline. Standalone CLI via cargo-dist is a later secondary channel. |
| Account switching | **Keychain hot-swap** (claude-swap semantics) | Transparent — plain `claude` keeps working. Accepts the engineering delicacy (locking, rotation) as a feature of the project. |
| First vendor | **Claude only**, behind a `Provider` trait | Codex is v2 (nearly identical mechanics), Cursor v3 (hacky/proven), Grok last (least stable). |
| Distribution | GitHub Releases → Developer ID + notarization + Sparkle → own brew tap → homebrew-cask at notability | Mac App Store is impossible (sandbox can't read Claude Code's keychain item). |

## How the data works (research findings)

Two sources for real "X% used, resets at HH:MM" (never estimate from token counts):

1. **Statusline stdin** (official, ToS-clean, primary). Claude Code pipes `rate_limits`
   JSON (`five_hour` / `seven_day` / `spend_limit`, each `used_percentage` + `resets_at`,
   epoch seconds) to the configured statusline script. tokache installs a shim that tees
   this into the state store. Pro/Max only; appears after first API response in a session;
   only covers the *active* account.
2. **`GET https://api.anthropic.com/api/oauth/usage`** (what `/usage` uses; undocumented).
   Headers: `Authorization: Bearer <accessToken>`, `anthropic-beta: oauth-2025-04-20`,
   and a `claude-code/<version>` User-Agent (without it → aggressive 429 bucket).
   Returns `five_hour` / `seven_day` / `seven_day_opus` / `seven_day_sonnet` / `extra_usage`
   with `utilization` + ISO `resets_at`. **Only way to poll idle accounts.** Poll gently,
   cache hard, degrade gracefully.
   - Companion endpoints: `GET /api/oauth/profile` (identity), token refresh at
     `POST https://platform.claude.com/v1/oauth/token` (Claude Code's public client_id
     `9d1c250a-e61b-44d9-88ed-5944d1962f5e`; refresh may rotate the refreshToken).

**Credential storage (macOS, verified):** one login-keychain item, service
`Claude Code-credentials`, account `$USER`. JSON blob:
`{ claudeAiOauth: { accessToken, refreshToken, expiresAt(ms), scopes, subscriptionType, rateLimitTier }, mcpOAuth: {...} }`.
Swap **only** `claudeAiOauth`; clobbering `mcpOAuth` destroys MCP server logins.

**Swap gotchas** (replicate claude-swap's handling — study `realiti4/claude-swap`
`src/claude_swap/{macos_keychain,credentials,oauth,usage_store,autoswitch}.py`):

- Hold Claude Code's credential lock while writing (race vs. its own token refresh).
- Write via `security add-generic-password -U` with secret on stdin, never argv.
- Claude Code caches keychain reads ~30s → swap applies on next refresh, not instantly.
- Refresh-token rotation: after Claude Code refreshes, re-capture that account's backup
  or the stored copy dies permanently (→ forced `/login`). The live keychain is the
  writer of record; backups are snapshots that must chase it.

**⚠️ ToS posture (be honest in the README):** Anthropic's Feb 2026 policy bans
subscription OAuth use "outside Claude Code and Claude.ai". Usage polling + account
management is a gray zone (not inference; same behavior as CCSeva/claude-swap, both
public). Mitigations: statusline is the primary data source (zero exposure); endpoint
polling is low-frequency and cached; tokache frames itself as managing *legitimately
separate* accounts (personal + work), not limit evasion.

## Architecture

```
Tokache.app (Swift, LSUIElement, SMAppService login item)
├── menubar UI ······· NSStatusItem + NSPopover shell, SwiftUI + Swift Charts panel
│                       (iStat-style gauges; FluidMenuBarExtra-quality behavior)
├── notifications ···· UserNotifications: "sub A at 92%", "sub B resets in 2h, unused"
├── Contents/Helpers/tokache ·· the Rust binary, symlinked to /usr/local/bin
└── reads/writes ⇅
    ~/Library/Application Support/tokache/
    ├── state.json ··· current snapshot (schema_version, accounts[], windows[], resets)
    ├── history.db ··· SQLite time series → charts
    └── accounts/ ···· per-account credential backups (encrypted / keychain-stored)

tokache CLI (Rust: clap + serde + rusqlite + reqwest)
├── tokache accounts add|list|remove   capture current login as a named account
├── tokache status                     gauges in the terminal (reads state, refreshes if stale)
├── tokache switch <name>              keychain hot-swap (claudeAiOauth only, locked)
├── tokache watch                      poll loop + auto-swap at threshold (hysteresis + cooldown)
├── tokache statusline                 the shim: tee rate_limits → state, passthrough output
└── tokache doctor                     diagnose keychain access, shim install, stale backups

Poller: the app while it runs (spawns/uses the CLI); `tokache watch` under launchd
        (SMAppService.agent) only if the user wants headless operation.
Provider trait in the Rust core: Claude first; Codex/Cursor/Grok implement later.
```

## Milestones

- **M0 — Rust CLI skeleton.** Cargo workspace (`core` lib + `cli` bin). Keychain
  read/parse (`security` or the `security-framework` crate), `accounts add`,
  `status` hitting oauth/usage with pretty terminal gauges. *(First Rust learning chunk.)*
- **M1 — State store + statusline shim.** SQLite history, JSON snapshot with
  `schema_version`, `tokache statusline` shim + installer, staleness rules
  (statusline data wins while fresh; endpoint fills idle accounts).
- **M2 — Manual switch.** `tokache switch` with full claude-swap semantics:
  lock, `claudeAiOauth`-only replacement, `mcpOAuth` preservation, backup
  re-capture on rotation, `doctor` checks.
- **M3 — Auto-swap + reminders.** `tokache watch`: threshold (default ~90%),
  best-headroom strategy, hysteresis + cooldown; "quota resets soon and you
  haven't used it" nudges (terminal-notifier/osascript until the app exists).
  ← **MVP complete: features 1, 2, 3 for Claude.**
- **M4 — Swift menubar app.** Xcode project (XcodeGen), NSStatusItem + NSPopover +
  SwiftUI panel, Swift Charts gauges from `state.json`/`history.db`, bundles the
  Rust binary, "Install CLI" onboarding + keychain-prompt explainer,
  native notifications. *(First Swift learning chunk.)*
- **M5 — Ship it.** Apple Developer Program ($99) → Developer ID cert → GitHub
  Actions: cargo build → xcodebuild → codesign (hardened runtime) → notarytool →
  staple → create-dmg → Sparkle appcast (generate EdDSA keys **before v1**) →
  GitHub Release + own brew tap (cask). Crib pipelines from Maccy / MonitorControl /
  Terzi's espanso guide.
- **M6 — Expand.** WidgetKit extension; Codex provider (`~/.codex/auth.json` swap,
  backend rate_limits — study steipete/CodexBar); then Cursor, then Grok.
  homebrew-cask submission at notability (75★ via third party / 225★ self).

## Key references

- claude-swap (swap semantics): https://github.com/realiti4/claude-swap
- Statusline docs (rate_limits schema): https://code.claude.com/docs/en/statusline
- CCSeva (Swift menubar Claude tracker, Electron→Swift migration): https://github.com/Iamshankhadeep/ccseva
- CodexBar (Codex+Claude menubar, closest comp): https://github.com/steipete/CodexBar
- claude-powerline (statusline-only UX proof): https://github.com/Owloops/claude-powerline
- Stats / Ice (UI polish benchmarks): https://github.com/exelban/stats · https://github.com/jordanbaird/Ice
- Release pipelines: https://github.com/p0deje/Maccy · https://github.com/MonitorControl/MonitorControl ·
  https://federicoterzi.com/blog/automatic-code-signing-and-notarization-for-macos-apps-using-github-actions/
- cargo-dist (standalone CLI channel, later): https://github.com/axodotdev/cargo-dist

## Signing note (do this early)

Reading Claude Code's keychain item triggers a macOS consent prompt. "Always Allow"
persists only under a **stable Developer ID signing identity** — ad-hoc builds
re-prompt after every update, and Claude Code's own credential rewrites can reset
the ACL anyway. Get the Developer ID before sharing builds; handle denial/`errSecAuthFailed`
gracefully; explain the prompt in onboarding before triggering it.
