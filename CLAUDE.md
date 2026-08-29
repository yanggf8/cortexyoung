# CLAUDE.md

This repo builds `cort`, an offline code-intelligence CLI over `ast-grep` + SQLite.

**The repo is pure Rust. No JavaScript, TypeScript, Python or other scripting language may exist as executable code** — not as a product entry point, not as tooling, not as tests. The eval harness that used to be six `.mjs` files was ported into the `evals/` crate for exactly this reason. Bash stays only where the platform requires it (`install.sh`, `tests/install-smoke.sh`); it is not a place to put logic. If a task seems to need a script, add a Rust subcommand to `evals/` or `rust/` instead.

Routing for day-to-day work is in `skills/ast-grep/SKILL.md`. `ast-grep` is the only parser: never add an in-process parser, never call the `sg` binary, and never loosen the `0.45.2` version pin.

Before every commit run `cargo test --all-targets` in **both** `rust/` (the product) and `evals/` (the harness), plus `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings` in each. Run `bash tests/install-smoke.sh` too when touching `install.sh`. If `cort` or `ast-grep` is missing, point the user to `./install.sh` — do not install automatically.
