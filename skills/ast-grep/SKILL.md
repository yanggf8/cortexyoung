---
name: ast-grep
description: Route code lookup, reusable reads, structural search, and impact questions to rg, ast-grep, or cort
---

Route by the narrowest output that answers the question. In 1,565 real user prompts, none asked for a code relationship graph, while file finding and reading consumed 61.5% of tool-output tokens. On a 27,296-estimated-token Rust file, `cort context main --content full -f lean` returned the exact function in 89 tokens (99.67% less); an unchanged repeat read returned a 21-token receipt instead of 27,295 tokens (99.92% less). These are separate savings, not additive.

Use this order for everyday work:

1. **Read one known function or method in a large indexed Rust file** — `cort context '<symbol>' --content full -f lean`. For methods, qualify the owner as `Type::method` (or `crate::path::Type::method`) to distinguish same-named methods. Check the header for `resolution=exact_symbol`; if an unqualified name returns multiple seeds, do not assume which one is intended.
2. **Read a file or line range you may need again** — `cort read <file> [--start N] [--end N] -f lean`. The first default read returns and persists the body. An unchanged repeat of the same range defaults to a one-line `source=store content=receipt` with no body; add `--content full` when you need the body again. Search only prior readings with `cort recall <query> -f lean`; add `--content full` for the complete stored fragment. Both commands validate the source, and require one prior `cort index`.
3. **Find a string, locate an unknown definition, or inspect a fresh edit** — `rg`. It sees working-tree changes immediately and is the right default for literal search. For the same literal string many times over a large repo, use `xg "PATTERN" --max-count 20` only when `command -v xg` succeeds.
4. **Trace callers or blast radius** — only for an explicit relationship question, use `cort impact --symbol <name[,name2]> --depth <n> -f lean`. Use `--depth 1` for direct callers and the default depth 3 for a genuinely transitive question. Do not route ordinary lookup here merely because a graph is available.

For structural syntax questions, use `ast-grep run -p '<pattern>' --lang <lang>` for one shape in one language. Use `cort struct -p '<pattern>' --lang <lang> -f lean` only when the answer also needs the enclosing symbols and their neighbours.

Always pass `-f lean`: it gives the same answer at about a fifth of JSON's tokens. Omit it only when machine-parseable JSON is needed.

Before trusting indexed answers, check `stale=` in lean output or `index_is_stale` in JSON. If true, run `cort index --incremental` first or fall back to `rg`. A new untracked file is invisible until indexed. `cort read` and `cort recall` do not use that field; they validate stored readings against the source instead.

Relationship resolution is name-based. A common unqualified name such as a logging or fetch helper can become a hub and pull in same-named symbols from unrelated files. Prefer owner-qualified Rust methods, spot-check depth-1 results, and treat deeper hops as candidates to confirm rather than verdicts.

The binary is `ast-grep`, never `sg`: on Linux `/usr/bin/sg` is setgroups(1), a different program.
