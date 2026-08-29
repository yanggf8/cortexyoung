# AGENTS.md

Instructions for any coding agent working in this repository (Codex reads `AGENTS.md`; Claude Code reads `CLAUDE.md`, which is a symlink to this file so the two can never drift).

This repo builds `cort`, an offline code-intelligence CLI over `ast-grep` + SQLite.

**The repo is pure Rust. No JavaScript, TypeScript, Python or other scripting language may exist as executable code** — not as a product entry point, not as tooling, not as tests. The eval harness that used to be six `.mjs` files was ported into the `evals/` crate for exactly this reason. Bash stays only where the platform requires it (`install.sh`, `tests/install-smoke.sh`); it is not a place to put logic. If a task seems to need a script, add a Rust subcommand to `evals/` or `rust/` instead.

Everything executable is Rust except two files bash actually requires: `install.sh` and
`tests/install-smoke.sh`. The ast-grep test double is a Rust bin (`fake_ast_grep`, declared in
`rust/Cargo.toml` and covered by `rust/tests/fixture.rs`), always built so `cargo test` needs no
special feature and never installed — `tests/install-smoke.sh` asserts the payload ships nothing
but `cort` and the pack.
