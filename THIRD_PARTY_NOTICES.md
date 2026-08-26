# Third-Party Notices

This repository bundles or references the following third-party projects.
Only their licenses and attributions are listed here; their source is not
vendored.

## ast-grep

- **Project:** https://github.com/ast-grep/ast-grep
- **Version pin:** `0.45.2` (exact; never loosen — the only parser)
- **License:** MIT
- **Usage:** Pinned, installed dependency. Prebuilt `app-<target>.zip` downloaded from GitHub Releases (`https://github.com/ast-grep/ast-grep/releases/download/0.45.2/app-<target>.zip`, where `<target>` is `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, or `aarch64-apple-darwin`) and verified against repo-maintained SHA-256 hashes in `install.sh` (upstream publishes no checksums; fail-closed on empty or mismatched hash). Fallback: `cargo install ast-grep --version 0.45.2 --locked` (requires Rust 1.88+). Binary is `ast-grep`, never `sg` (on Linux `/usr/bin/sg` is `setgroups(1)` and is a different program).

## better-sqlite3

- **Project:** https://github.com/WiseLibs/better-sqlite3
- **License:** MIT
- **Usage:** The only runtime npm dependency of `cort` (`"better-sqlite3": "^11.10.0"` in `package.json`). Provides the SQLite binding for the per-project index. Installed via `npm ci --omit=dev` into `~/.local/share/cortexyoung/cort` by `install.sh`.

## ripgrep (rg)

- **Project:** https://github.com/BurntSushi/ripgrep
- **License:** MIT OR Unlicense
- **Usage:** Companion tool referenced in README and `skills/ast-grep/SKILL.md`; not installed by this repo. Expected to be present on the host or embedded in Claude Code.

## xgrep / xg (optional)

- **Project:** https://github.com/momokun7/xgrep
- **Crate:** `xgrep-search` v0.7.0 on crates.io (note: `xgrep` on crates.io is a different, unrelated crate)
- **License:** MIT OR Apache-2.0 (upstream dual-licensed)
- **Usage:** Optional `--with-xgrep` extra. Not installed by default. When requested, `xg` binary is downloaded as a pinned prebuilt `xg-<target>.tar.gz` from GitHub Releases, verified against repo-maintained SHA-256 in `install.sh`, or built via `cargo install xgrep-search --version 0.7.0 --locked`. The `skills/xgrep/SKILL.md` skill is deployed only with `--with-xgrep`. All upstream performance numbers are self-reported and not independently verified in this repo.

## Archived Cortex V6 dependencies (tag `v6-final` only)

The following were dependencies of the archived `cortex` CLI (preserved at `v6-final` and no longer installed by this repo):

- `@xenova/transformers` (BGE-small-en-v1.5) — Apache-2.0
- `@libsql/client` / Turso — MIT
- `web-tree-sitter` + `tree-sitter-*` grammars — MIT
