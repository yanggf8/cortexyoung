# Third-Party Notices

This repository bundles or references the following third-party projects.
Only their licenses and attributions are listed here; their source is not
vendored.

## xgrep / xg

- **Project:** https://github.com/momokun7/xgrep
- **Crate:** `xgrep-search` v0.7.0 on crates.io (note: `xgrep` on crates.io is a different, unrelated crate)
- **License:** MIT OR Apache-2.0 (upstream dual-licensed)
- **Usage:** `xg` binary downloaded as a pinned prebuilt from GitHub Releases, or built via `cargo install xgrep-search --version 0.7.0 --locked`. SHA-256 hashes in `install.sh` are repo-maintained; upstream publishes no checksums. All upstream performance numbers are self-reported and not independently verified in this repo.

## ripgrep (rg)

- **Project:** https://github.com/BurntSushi/ripgrep
- **License:** MIT OR Unlicense
- **Usage:** Companion tool referenced in README; not installed by this repo. Expected to be present on the host or embedded in Claude Code.

## ast-grep (sg / ast-grep)

- **Project:** https://github.com/ast-grep/ast-grep
- **License:** MIT
- **Usage:** Companion tool referenced in README for structural/AST search and `--rewrite` refactoring; not installed by this repo.

## Archived Cortex V6 dependencies (tag `v6-final` only)

The following were dependencies of the archived `cortex` CLI (preserved at `v6-final` and no longer installed by this repo):

- `@xenova/transformers` (BGE-small-en-v1.5) — Apache-2.0
- `@libsql/client` / Turso — MIT
- `web-tree-sitter` + `tree-sitter-*` grammars — MIT
