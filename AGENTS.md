# AGENTS.md

Coding standards: see `CODING_STANDARDS.md`.

## Tooling

No Rust toolchain in the ambient PATH; always go through devenv. Everything below is a script in `devenv.nix`, run as `devenv shell -- <script>`.

- `devenv shell -- format` — `cargo fmt --all` over the workspace. Run it after every Rust edit. Arguments pass through, so `devenv shell -- format --check` reports without rewriting.
- `devenv shell -- lint` — the gate before reporting work done. Three steps:
  1. `cargo fmt --all --check` — code must already be formatted.
  2. `cargo clippy --workspace --lib --bins -- --deny warnings` — pedantic and nursery lints, plus `unwrap_used` and `allow_attributes_without_reason` (every `#[allow]` needs `reason = "..."`).
  3. A grep that rejects any `fn get_*` in `core/` or `actions/` — name getters after the value (`album_json`, not `get_album_json`).
- `devenv shell -- check` — fast type check: `cargo check --workspace`, then the web app's `bun run check` (svelte-check) and `bun run lint:logic` (eslint).
- `devenv shell -- tests` — `cargo test --workspace`, then `bun run test` (bun unit tests) and `bun run lint` for the web app (svelte-check, eslint, stylelint).
- `devenv shell -- build` — artifacts only: `cargo build --release` for `datamile` and the action binaries, refresh the `actions/` python symlinks, then the web app's `vite build`. It runs no checks; use `lint`, `check` and `tests` for those.

Never call `cargo fmt` or `cargo clippy` directly. If `lint` fails, fix the code. Do not relax the lint.

One shell collision to keep in mind: the script is `tests`, not `test`, because coreutils' `test` shadows it in the shell. The formatter script is `format` for the same reason — coreutils ships a `fmt`.

## Agent skills

### Issue tracker

None configured yet; specs and runbooks live locally in `specs/`. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context (`GLOSSARY.md` and `docs/adr/` at repo root). See `docs/agents/domain.md`.
