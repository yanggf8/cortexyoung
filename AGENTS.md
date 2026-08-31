# AGENTS.md

Instructions for any coding agent working in this repository (Codex reads `AGENTS.md`; Claude Code reads `CLAUDE.md`, which is a symlink to this file so the two can never drift).

This repo builds `cort`, an offline code-intelligence CLI over `ast-grep` + SQLite.

**The product's long-term goal is one sentence, and it is a measurement rather than a sentiment: make
the caller-set enumeration an agent already performs — and often gets wrong — both cheap and
checkable.** Do not justify graph work by saying users ask relationship questions. They do not: on the
surviving local corpus (`cort-evals demand`, `docs/2026-08-31-demand-recheck.md`) 1,214 genuine user
instructions held **one** relational question (0.08%), and 4-7 (0.33-0.58%) were instructions that
cannot be done correctly without a call-site set - all of them on the delete / refactor / review path.
The same corpus shows 42% of what arrives as a "user message" is a pasted agent report being fact-checked,
which is the real surface: the agent does this work unprompted, gets multi-hop answers wrong in 6 of 10
cells (`evals/runs/2026-08-30-graph{,-sample2}/`), and is then asked to prove it. Cost per use is already
settled (7.7x smaller tool payload at ~4x fewer turns, same venue, same task set); **checkability is the
open half** - `impact` reports each dependent's *definition* line and not its *call site*, so confirming
one edge still costs a re-read, and `relationships` stores no call-site column. A change that makes an
answer cheaper to verify is on the main line; a feature that only makes answers more numerous is not.

**The repo is pure Rust. No JavaScript, TypeScript, Python or other scripting language may exist as executable code** — not as a product entry point, not as tooling, not as tests. The eval harness that used to be six `.mjs` files was ported into the `evals/` crate for exactly this reason. Bash stays only where the platform requires it (`install.sh`, `tests/install-smoke.sh`); it is not a place to put logic. If a task seems to need a script, add a Rust subcommand to `evals/` or `rust/` instead.

**No absolute paths from any developer's machine, and no Node-installed toolchain paths, anywhere in
the repo — including test fixtures and fallbacks.** `ast-grep` is provisioned by `install.sh` from the
pinned native release asset (or `cargo install`); a test that cannot find it prints `SKIP:` instead of
reaching for a host-specific binary.

**A `skills/<name>/SKILL.md` is deployed byte-for-byte.** Nothing — not an ownership marker, not a
comment, not a banner — may be inserted into it, because two third-party parsers own that format and
the frontmatter key set is closed. `install.sh` claims ownership in `.cortexyoung-managed` beside the
file, recording the SHA-256 of the bytes it wrote; `rust/tests/skill_format.rs` gates the source shape
and `tests/install-smoke.sh` gates the deployed shape.

Everything executable is Rust except two files bash actually requires: `install.sh` and
`tests/install-smoke.sh`. The ast-grep test double is a Rust bin (`fake_ast_grep`, declared in
`rust/Cargo.toml` and covered by `rust/tests/fixture.rs`), always built so `cargo test` needs no
special feature and never installed — `tests/install-smoke.sh` asserts the payload ships nothing
but `cort` and the pack.
