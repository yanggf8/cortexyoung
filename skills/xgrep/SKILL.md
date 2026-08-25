---
name: xgrep
description: Indexed trigram search via xg — use for repeated identifier/content search in medium-to-large repos; complements native Grep.
---

# xgrep — indexed search

`xg` is a trigram inverted index (Rust). Use it to accelerate repeated searches on large trees. Fall back to native Grep when `xg` is absent or the task is small.

## Rules

1. **Repeated identifier/content search in a large repo** → `xg "PATTERN" --max-count 20` (DEFAULT format; do not use `--format llm` by default — it costs ~3.3× tokens).
2. **Just-edited tracked files** → `xg "PATTERN" --changed`. Brand-new untracked files are invisible to both `--changed` and `--fresh` (verified in `xgrep` `git.rs:90` / `updater.rs:193`) — use native Grep for those.
3. **Patterns <3 chars, very common substrings, small repos, or one-off searches** → native Grep (`rg`).
4. **Filename search** → `xg --find "GLOB"` (staler than content search: skips hybrid overlay and background rebuild kick).
5. **First search in a large tree may block on `[indexing...]`** — let it finish; subsequent searches are near-instant.

## Quick reference

```bash
xg "pattern" --max-count 20        # content search, smart-case, capped
xg --find "*.chunker*"             # filename search (glob)
xg "pattern" --changed             # only git-dirty tracked files
xg status                          # index status; xg init to force rebuild
```
