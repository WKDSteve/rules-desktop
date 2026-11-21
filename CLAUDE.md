# rules-desktop

A local web UI and desktop app for the rule set of rules.nvim: the hub (`crates/rules-hub`, Rust on appkit) and the
browser interface (`ui/`, Svelte 5, Vite, TypeScript). What it is and why: [README.md](README.md); usage, security,
processes and releasing: [docs/](docs/).

## What an agent must know

- **The engine is the only writer.** The app never parses or formats a rule block; it talks to rules.nvim through
  `rules-export/1` and `rules-ops/1` only. `ui/src/lib/contract.d.ts` is a copy of the engine's schema (CI compares
  them); `engine.lock` pins the engine.
- **Rule text is shown as text** and a rule's `check` is never run. Loopback only, token always (see
  [docs/SECURITY.md](docs/SECURITY.md)).
- **Every program the hub starts** is listed in [docs/PROCESSES.md](docs/PROCESSES.md); `tools/check-process-starts.py`
  enforces it.
- **Colours** are tokens of appkit-ui or of `ui/src/palette.css` (generated, never edited here);
  `tools/check-color-literals.py` refuses literals. No inline script, no `data:` URL (CSP `'self'`).
- **appkit** is pinned by one tag in `Cargo.toml` and `ui/package.json`; `tools/check-appkit-pin.py` checks both lock
  files. The `github:` dependency needs codeload.github.com for `pnpm install`.
- **Temp folders** in tests come from `Scratch` (Rust) or `ui/e2e/cleanup.ts`, never named by hand.
- **Checks** (see `.github/workflows/ci.yml`): `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `cargo test --workspace --locked`; in `ui/`: `pnpm install --frozen-lockfile`,
  `pnpm check`, `pnpm test`, `pnpm build`, `pnpm e2e`; and `python -m unittest discover tools`,
  `python tools/check-control-chars.py`, `python tools/check-process-starts.py`, `python tools/check-color-literals.py
  --root ui/src --tokens ui/src/palette.css`. Manual test sheets: `TESTS/sheets/` with `.testing.lua` (testing.nvim).

## Web platform
- Browser Support: Baseline Widely available (`.mwgrc`, `browserslist`). Newer features only behind feature detection with a fallback; polyfills where browsers still in use need them (rather one too many), run only after the check: dynamic `import()`, or bundled in a `file://` page (rules-vault `BASE-02`).
- Before writing or changing HTML, CSS or client-side JS, use the `modern-web-guidance` skill (`search` → `retrieve` → verify). Not for Rust, backend, CI or build config.
- For open-ended modernisation: `npx -y modern-web-guidance@latest legacy-scan ui/src --json`, check each finding against `keepWhen`/`dropWhen`.
- Rules: rules-vault ruleset `web` (plus the app rules `WEB-*` in ruleset `native`).
- The scanner is an exact devDependency of `ui` (the policy of `ui/pnpm-workspace.yaml` applies):
  `cd ui && DISABLE_TELEMETRY=1 pnpm exec modern-web-guidance legacy-scan src --json`; CI runs it as an informative
  step. `.claude/settings.json` turns the tool's telemetry off and allows only `npx -y modern-web-guidance@latest *`.
