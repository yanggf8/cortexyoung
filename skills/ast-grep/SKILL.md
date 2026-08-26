---
name: ast-grep
description: Route structural code questions to ast-grep and cort instead of grepping and reading files
---

Pick the narrowest tool that answers the question.

- **Fresh edits, short patterns, small repos** — native Grep (`rg`). `cort`'s index lags an unsaved edit; `rg` never does.
- **One structural shape, one language, no cross-file context needed** — `ast-grep run -p '<pattern>' --lang <lang>`.
- **The same shape plus who touches it** — `cort struct -p '<pattern>' --lang <lang>`. Same matches, each joined to its enclosing symbol with up to 3 graph neighbours. `--lang` is required: it drives the pattern pre-flight that turns a malformed pattern into `{"error":"parse_failed"}` instead of a silent empty result.
- **"What else deals with X"** — `cort context <symbol-or-query>`. Exact symbol first, keyword recall otherwise, then depth-1 neighbours, budgeted to about 1500 tokens.
- **"What breaks if I change X"** — `cort impact --symbol <name>`. Reverse dependents to depth 3.
- **Repeated literal-string search over a large repo** — `xg "PATTERN" --max-count 20`, only when `command -v xg` succeeds.

Every `cort` command returns JSON with `index_is_stale`. When it is `true`, run `cort index --incremental` before trusting the answer, or fall back to `rg`.

`cort index` must have been run once for the project. A brand-new untracked file is invisible to `cort` until the next index.

The binary is `ast-grep`, never `sg`: on Linux `/usr/bin/sg` is setgroups(1) and is a different program.
