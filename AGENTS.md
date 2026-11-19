# AGENTS.md

## Coding Standards

- Avoid magic numbers and strings by extracting recurring or meaningful values into descriptive constants (const) or enums. Keep self-explanatory, one-off values inline to avoid clutter. If a value comes from a spec (e.g. HTTP 200 OK), use a constant regardless.

- Reduce code indentation. Avoid Arrow Anti-Pattern. Leverage early return and continue.

- Use enums instead of booleans for function parameters.

- Let the reader of the code breathe. Add empty lines between logical blocks of code.

- Always document tests. Be concise and to the point.

- ALWAYS document `#[allow]` blocks. Use them only if good reason exists. No exceptions.

- Add a small, to the point, comment to explain *why* the block does what it does. Use examples when possible.

- Use descriptive self-documenting function names that make clear what the function does from the name only.

- Program to levels of abstraction. Lower-level mechanics (e.g., raw hardware I/O, sector parsing, direct socket streams) must be encapsulated in a dedicated driver/abstraction layer. Expose clean, high-level APIs to the rest of the application so calling code works with domain concepts, not raw implementation details.

- Don't touch blocks of code unrelated to the feature you implement. e.g. Don't add comments to a block of code if you did not create it or modify it. As much as possible try to minimize the number of changed lines when implementing a feature.

- Strictly adhere to the layered boundary hierarchy: each layer may only communicate with its immediate neighbor directly below it. Never "punch holes" through layers (e.g., controllers or UI components must never directly call database queries, raw hardware drivers, or low-level network clients; always route through the intermediate service/abstraction layer).

- Always document Rust tests in files dedicated to them. Be concise and to the point.

- Always use {}, even on a one-line "if" statement.

- If the bug was found while you were working, or the prompt itself indicates that a bug is being fixed, don't write the fix right away. First write the test. Observe it failing. Then write the fix. And then observe the test passing.

- If the prompt indicates that a bug is being fixed, don't write the fix right away. First write the test. Observe it failing. Then write the fix. And observe the test passing.

## Tooling

No Rust toolchain in the ambient PATH; always go through devenv. Everything below is a script in `devenv.nix`, run as `devenv shell -- <script>`.

- `devenv shell -- fmt` — `cargo fmt --all` over the workspace. Run it after every Rust edit. Arguments pass through, so `devenv shell -- fmt --check` reports without rewriting.
- `devenv shell -- lint` — the gate before reporting work done. Three steps:
  1. `cargo fmt --all --check` — code must already be formatted.
  2. `cargo clippy --workspace --lib --bins -- --deny warnings` — pedantic and nursery lints, plus `unwrap_used` and `allow_attributes_without_reason` (every `#[allow]` needs `reason = "..."`).
  3. A grep that rejects any `fn get_*` in `core/` or `actions/` — name getters after the value (`album_json`, not `get_album_json`).
- `devenv shell -- check` — fast type check: `cargo check --workspace`, then the web app's `bun run check` (svelte-check) and `bun run lint:logic` (eslint).
- `devenv shell -- tests` — `cargo test --workspace`, then `bun run lint` for the web app (svelte-check, eslint, stylelint).
- `devenv shell -- build` — full pass: clippy, tests, `cargo build --release`, refresh the `actions/` python symlinks, then web-app check, eslint and `vite build`.

Never call `cargo fmt` or `cargo clippy` directly. If `lint` fails, fix the code. Do not relax the lint.

Two shell collisions to keep in mind: the script is `tests`, not `test`, because coreutils' `test` shadows it in the shell; and `devenv.nix` prepends the `fmt` script to `PATH` in `enterShell` so it beats coreutils' `fmt`.
