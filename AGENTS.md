# AGENTS.md

Instructions for any coding agent working in this repository (Codex reads `AGENTS.md`; Claude Code reads `CLAUDE.md`, which is a symlink to this file so the two can never drift).

This repo builds `cort`, an offline code-intelligence CLI over `ast-grep` + SQLite.

**The repo is pure Rust. No JavaScript, TypeScript, Python or other scripting language may exist as executable code** — not as a product entry point, not as tooling, not as tests. The eval harness that used to be six `.mjs` files was ported into the `evals/` crate for exactly this reason. Bash stays only where the platform requires it (`install.sh`, `tests/install-smoke.sh`); it is not a place to put logic. If a task seems to need a script, add a Rust subcommand to `evals/` or `rust/` instead.

Known exception, not to be grown: `rust/tests/fixtures/fake-ast-grep` is a 46-line python stand-in used by
five pathological parser tests. It is tracked as F-14 in
`docs/2026-08-29-project-audit-root-causes-and-remediation.md` and should become a Rust test fixture
binary; until then do not add anything else in a scripting language, and do not treat that file as
precedent.

Routing for day-to-day work is in `skills/ast-grep/SKILL.md`. `ast-grep` is the only parser: never add an in-process parser, never call the `sg` binary, and never loosen the `0.45.2` version pin.

Before every commit run `cargo test --all-targets` in **both** `rust/` (the product) and `evals/` (the harness), plus `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings` in each. Run `bash tests/install-smoke.sh` too when touching `install.sh`. If `cort` or `ast-grep` is missing, point the user to `./install.sh` — do not install automatically.

## Layout

- `rust/` — the product crate (`cort`) and its tests. 223 tests.
- `evals/` — the dev-only `cort-evals` crate: the agent-eval harness. Never installed, never imported by the product.
- `skills/ast-grep/SKILL.md` — the one routing guide agents are meant to follow. `install.sh` deploys it to both `~/.claude/skills/` and `~/.codex/skills/`, so changing routing means changing this file, not the installed copies.

## Guidance is a measured artefact

`skills/ast-grep/SKILL.md` is injected into every session that uses it, so its size is a cost and its
claims are load-bearing. When you change what cort does — routing, staleness, supported languages,
cost — update that file in the same change, and keep it net-shorter where possible. Numbers quoted
there must trace to a run in `evals/runs/` or a document under `docs/`, with its caveats (n, venue,
HEAD, and whether the arm was actually contained).
