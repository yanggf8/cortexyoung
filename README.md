# cortexyoung — xgrep-centered code search

Thin repo that makes `xg` (xgrep, trigram inverted index) available as a Claude Code skill. No embeddings, no cloud DB, no servers — one binary, one skill, one install script.

## Install

```bash
./install.sh              # prebuilt xg v0.7.0 + skill to ~/.claude/skills/xgrep/
./install.sh --check      # verify without mutating
./install.sh --uninstall  # remove managed artifacts only
./install.sh --force      # on unmanaged skill collision: backup and replace
./install.sh --with-rustup # bootstrap rustup if cargo is missing
```

**What it does:**

- Downloads the pinned `xg` v0.7.0 prebuilt for your platform (Linux x86_64/aarch64, macOS x86_64/arm64) from GitHub Releases and verifies SHA-256 (repo-maintained; upstream publishes no checksums). Falls back to `cargo install xgrep-search --version 0.7.0 --locked` (note: crate is `xgrep-search`, not `xgrep` — name collision on crates.io).
- Deploys `skills/xgrep/SKILL.md` to `~/.claude/skills/xgrep/SKILL.md` with a managed marker. Preflights collisions before mutating: skips if hash-equal, replaces if managed, refuses unmanaged collisions (use `--force` to backup and replace).
- Adds a single bounded idempotent `PATH` block to your shell profile (`.bashrc`/`.zshrc`/`.profile`) so `xg` is on `PATH`; removed on `--uninstall`.
- Records ownership in `~/.local/share/cortexyoung/manifest` — uninstall only removes what it installed, never a pre-existing `~/.cargo/bin/xg`.

**Requirements:** `curl` or `wget`, `tar`, `sha256sum`/`shasum`. Rust toolchain only needed for the cargo fallback.

## Update

```bash
git pull --no-rebase
./install.sh              # idempotent — skips hash-equal skill, no duplicate PATH block
```

## Uninstall

```bash
./install.sh --uninstall  # removes managed skill + PATH block + manifest-owned xg binary
```

Pre-existing `xg` binaries and unmanaged skills are never removed.

## How the skill is used

The `xgrep` skill teaches agents when to use `xg` vs native Grep — see `skills/xgrep/SKILL.md` (also installed to `~/.claude/skills/xgrep/`). Key rules are summarized there; the five documented limitations below are the contract.

## Known limitations of `xg` (documented contracts)

1. **Staleness:** plain searches read the index silently and may serve stale results right after files change — no warning. After edits you made, use `--changed` or `--fresh`.
2. **Untracked files are invisible to `--changed` and `--fresh`:** both flags only cover git-tracked changes (`git diff --name-only` / `--cached`); brand-new untracked files are missed — use native Grep for those.
3. **Patterns under 3 characters bypass the index** (no trigram to look up) — falls back to a full scan and can be slower than `rg`.
4. **Very common substrings spread across many files** can be slower than `rg` due to posting-list fan-out.
5. **Filename search (`--find`) is staler** than content search — it skips the hybrid overlay and background rebuild kick.

## Performance notes

All upstream performance claims for `xg`/`xgrep` are self-reported by the upstream author; no independent third-party benchmarks are bundled with this repo. Measure on your own workload before relying on them.

## Archival access

Cortex V6 (AST chunking, embeddings, Turso, relationship graph, `cortex` CLI) is archived at tag `v6-final`:

```bash
git show v6-final:docs/2026-08-25-audit-and-repositioning.md
git show v6-final:CLAUDE.md
git show v6-final:cli/src/index.ts
```

## Companion tools (not installed by this repo)

These are recommended alongside `xg` for a fuller search toolkit, but are **not** part of this repo's install:

- **[ast-grep (`sg`/`ast-grep`)](https://ast-grep.github.io/)** — structural/AST search and `--rewrite` refactoring (`cargo install ast-grep --locked` or `npm i -g @ast-grep/cli`). Use when you need syntax-aware queries (e.g., find all `useState` calls with a specific pattern) rather than plain text search. Complements `xg`/`rg` rather than replacing them.
- **[ripgrep (`rg`) 15+](https://github.com/BurntSushi/ripgrep)** — already expected on most dev machines and embedded in Claude Code itself. Keep it updated (`cargo install ripgrep` or your system package manager). `xg` handles repeated large-tree queries; `rg` handles one-offs, short patterns, and untracked files.

## Upstream credits

- [`xgrep` (momokun7/xgrep)](https://github.com/momokun7/xgrep) — trigram inverted index, `xg` CLI (`xgrep-search` on crates.io), MIT/Apache-2.0.

## License

MIT — see [LICENSE](LICENSE). Third-party notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
