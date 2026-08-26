# cortexyoung — cort, offline code-intelligence over ast-grep + SQLite

`cort` is an offline code-intelligence CLI built on `ast-grep` (the only parser, never `sg`) and SQLite. One repo checkout, one SQLite index per project, no embeddings, no cloud DB, no servers.

Four shipped commands (plus `status`/`projects`/`delete` utilities):

- `cort index [--incremental] [path]` — build or incrementally refresh the index (`ast-grep` + SQLite)
- `cort struct -p '<pattern>' --lang <lang>` — structural search joined to enclosing symbols + 3 neighbours
- `cort context <symbol-or-query>` — "what else deals with X" (exact symbol or FTS recall, depth-1 neighbours, ~1500-token budget)
- `cort impact --symbol <name>` — "what breaks if I change X" (reverse dependents, depth 3)

Routing for agents is in `skills/ast-grep/SKILL.md` — it states when to use `rg`, `ast-grep run`, `cort struct`/`context`/`impact`, and `xg`.

## Install

```bash
./install.sh              # cort v0.1.0 + ast-grep v0.45.2 + skill to ~/.claude/skills/ast-grep/
./install.sh --check      # verify without mutating
./install.sh --uninstall  # remove managed artifacts only (reads manifest v2)
./install.sh --force      # on unmanaged skill collision: backup and replace
./install.sh --with-rustup # bootstrap rustup if cargo is missing
./install.sh --with-xgrep  # opt-in: also install xg v0.7.0 (xgrep-search crate) + xgrep skill
```

**What it does (default, without `--with-xgrep`):**

- Downloads pinned `ast-grep` v0.45.2 prebuilt `app-<target>.zip` for your platform (Linux x86_64/aarch64, macOS x86_64/arm64) from GitHub Releases and verifies SHA-256 (repo-maintained; upstream publishes no checksums) — fail-closed: an empty or mismatched checksum refuses to install. Falls back to `cargo install ast-grep --version 0.45.2 --locked` (requires Rust 1.88+).
- Installs `cort` from this checkout to `~/.local/share/cortexyoung/cort` via `npm ci --omit=dev` (Node >= 22, `better-sqlite3` is the only runtime dependency) and shims `~/.cargo/bin/cort` or `~/.local/bin/cort`.
- Deploys `skills/ast-grep/SKILL.md` to `~/.claude/skills/ast-grep/SKILL.md` with a managed marker. Preflights collisions before mutating: skips if hash-equal, replaces if managed, refuses unmanaged collisions (use `--force` to backup and replace).
- Adds a single bounded idempotent `PATH` block to your shell profile (`.bashrc`/`.zshrc`/`.profile`) so `cort` and `ast-grep` are on `PATH`; removed on `--uninstall`.
- Records ownership in `~/.local/share/cortexyoung/manifest` (v2 `key:value` lines) — uninstall only removes what it installed, never a pre-existing binary.

**Requirements:** `curl` or `wget`, `tar`, `unzip`, `sha256sum`/`shasum`. Node >= 22. Rust only needed for the cargo fallback.

## Update

```bash
git pull --no-rebase
./install.sh              # idempotent — skips hash-equal skill, no duplicate PATH block
```

With xgrep (opt-in):

```bash
./install.sh --with-xgrep # idempotent xg install + xgrep skill deploy
```

## Uninstall

```bash
./install.sh --uninstall  # removes managed cort + ast-grep binaries, CORT_HOME, skills, PATH block, manifest
```

Pre-existing binaries and unmanaged skills are never removed. With `--with-xgrep`, the managed `xg` binary and `xgrep` skill are also removed if owned.

## The ast-grep 0.45.2 pin — why fail-closed

`ast-grep` is the only parser (never add an in-process parser, never call `sg` — on Linux `/usr/bin/sg` is `setgroups(1)`). The pin is `0.45.2` exactly: the installer verifies the download's SHA-256 against repo-maintained hashes for `app-<target>.zip` before extracting. An empty expected hash is fatal (`no checksum on record`), and a mismatch is fatal (`refusing to install an unverified binary`). This is fail-closed because installing an unverified binary would silently change parse behaviour — `parse_failed` detection, pattern validation, and struct/context/impact all depend on the same parser version.

Alternative install (same pin, same fail-closed version check):

```bash
cargo install ast-grep --version 0.45.2 --locked  # requires Rust 1.88+
```

## Documented limitations (contracts, not apologies)

1. **Index staleness:** the index lags unsaved edits and brand-new untracked files. Every `cort` command returns JSON with `index_is_stale`; when `true`, run `cort index --incremental` before trusting the answer, or fall back to `rg`. A brand-new untracked file is invisible to `cort` until the next index. `cort index` must have been run once for the project.
2. **`chunk_id` stability:** `chunk_id` is stable only while a symbol's first line does not move — inserting lines above a symbol changes its id.
3. **`context` is FTS-only:** `cort context` uses SQLite FTS keyword recall, not semantic search or embeddings. No vector, no RRF, no reranking.
4. **Name-based target resolution:** relationship targets are resolved by symbol name. A same-named symbol in an unimported file can still surface as `AMBIGUOUS`, even if it is not actually imported.
5. **`--lang` is required on `struct`:** `cort struct -p '<pattern>' --lang <lang>` fails with `{"error":"missing_lang"}` if `--lang` is absent. It also drives the pattern pre-flight that turns a malformed pattern into `{"error":"parse_failed"}` instead of a silent empty result. The binary is `ast-grep`, never `sg`.

## What is deliberately not built

Until the eval harness verdict says otherwise (spec section 8), these are deferred and must not appear:

- `rewrite` (`cort rewrite` / `ast-grep --rewrite` wiring, dry-run, `--interactive`, `--update-all`)
- `modules` (`cort modules` Louvain Phase-1 greedy community detection)
- `--watch` (file-watcher with `inFlight` serialization)
- `impact --from-diff` (diff-aware blast radius)
- `search` as a first-class verb (`cort search` — use `cort struct` / `cort context` instead)
- `embeddings` / `cort embed --backfill` (`ALTER TABLE chunks ADD COLUMN embedding BLOB`, BGE, dense search, three-arm RRF)

## Archival access to V6

Cortex V6 (AST chunking, embeddings, Turso, relationship graph, `cortex` CLI) is archived at tag `v6-final`:

```bash
git show v6-final:docs/2026-08-25-audit-and-repositioning.md
git show v6-final:CLAUDE.md
git show v6-final:cli/src/index.ts
git show v6-final:README.md
```

Any path can be inspected via `git show v6-final:<path>` without checking out the tag.

## How the skill is used

The `ast-grep` skill teaches agents when to use `rg` vs `ast-grep` vs `cort` — see `skills/ast-grep/SKILL.md` (also installed to `~/.claude/skills/ast-grep/`). `rg` for fresh/short/small, `ast-grep run` for one shape, `cort struct` for shape + neighbours, `cort context` for neighbourhood, `cort impact` for blast radius, `xg` only when `command -v xg` succeeds and the task is repeated literal-string search.

## Upstream credits

- [`ast-grep`](https://github.com/ast-grep/ast-grep) v0.45.2 — MIT, installed from GitHub Releases `app-<target>.zip` (repo-maintained SHA-256) or `cargo install ast-grep --version 0.45.2 --locked`.
- [`better-sqlite3`](https://github.com/WiseLibs/better-sqlite3) — MIT, the only runtime npm dependency of `cort`.
- [`xgrep` (momokun7/xgrep)](https://github.com/momokun7/xgrep) — MIT/Apache-2.0, optional `--with-xgrep` extra (`xg` v0.7.0, `xgrep-search` on crates.io).
- [`ripgrep`](https://github.com/BurntSushi/ripgrep) — MIT OR Unlicense, not installed by this repo; expected on the host.

## License

MIT — see [LICENSE](LICENSE). Third-party notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
