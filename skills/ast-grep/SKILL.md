---
name: ast-grep
description: Route structural code questions to ast-grep and cort instead of grepping and reading files
---

`cort` answers **relationship** questions. It does not beat `rg` at finding text — measured on a 2,676-chunk TypeScript repo, answering a 3-hop blast-radius question costs `cort impact -f lean` about 1.1k tokens, while `rg` + reads to reach the same answer set costs about 125k. The same question at depth 1 costs about 0.9k vs 16k. The gap is the graph; do not use `cort` where there is no graph to walk.

Pick the narrowest tool that answers the question.

- **Find a string, a definition, or a fresh unsaved edit** — native Grep (`rg`). `cort`'s index lags; `rg` never does. If you would use `cort context <query>` on a name you already know, grep instead.
- **The same literal string many times over a large repo** — `xg "PATTERN" --max-count 20`, only when `command -v xg` succeeds.
- **One structural shape, one language, no cross-file context** — `ast-grep run -p '<pattern>' --lang <lang>`.
- **That shape plus who touches it** — `cort struct -p '<pattern>' --lang <lang> -f lean`.
- **"What breaks if I change X" / "who reaches this" / "what must change to remove X"** — `cort impact --symbol <name[,name2]> --depth <n> -f lean`. This is `cort`'s reason to exist: each extra hop costs `cort` one query and costs `rg` a fresh grep *plus* a read of every hit to learn the next hop's names. Use `--depth 3` (default) for real blast radius; `--depth 1` for direct callers.
- **"What else deals with X", narrowly** — `cort context <symbol-or-query> -f lean`, budgeted to about 1500 tokens. Pass `--content full` only when you need the whole body.

Always pass `-f lean`: it is the same answer at about a fifth of the tokens (one row per result, no ids). Omit it only when you need machine-parseable JSON.

Every `cort` command reports `stale=` in lean output, `index_is_stale` in JSON. When true, run `cort index --incremental` first, or fall back to `rg`. `cort index` must have been run once; a brand-new untracked file is invisible until then.

Relationships resolve by symbol name, so a common name (a logging or fetch helper) can be a hub and pull in same-named symbols from unimported files. Trust `--depth 1` sets you can spot-check; treat deeper hops as candidates to confirm, not verdicts.

The binary is `ast-grep`, never `sg`: on Linux `/usr/bin/sg` is setgroups(1) and is a different program.
