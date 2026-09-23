# 2026-09-24 — The empty-index boundary learns to speak

## The incident

At 2026-09-24 01:05 a sandboxed `cort impact` on travel-2026 (Codex Desktop) died against the
sandbox wall mid-create and left a schema-only database in the cache: 135 KB, `file_state` empty,
no files ever indexed. Travel-2026 had never had a real index on that machine — it was not in the
seven `cort-upgrade --check` knows about.

Every `impact` that touched the husk afterwards — the elevated rerun included — answered:

```
# impact DAY_ROADS depth=1 seeds=0 dependents=0 stale=true repair=rebuild_required
coverage  no_seed_resolved  not a clean answer: nothing was looked at
```

with **no heal marker of any kind**. Two agents misread that in sequence: Codex attributed the
failure to its sandbox (true for the create, not for the queries after), and the Claude Code
session that re-ran the same query elevated initially read the successful open as "the self-heal
fired" — it had not, and could not have: `heal.rs` no-opped on `indexed == 0` **silently**, by
design, on every query, forever. The repair was one explicit `cort index` (442 files, 7,170
relationships, 5.96s), which is the designed bootstrap lever and remains so.

## What was wrong

The never-create boundary itself is right and stays: a query repairs a cache, it never bootstraps
one, so a stray `context` in an arbitrary directory builds nothing. The defect was that the
boundary's silence sat beside `repair=rebuild_required` — a payload that names what the *hook*
would owe while saying nothing about what the *heal* did. A reader cannot distinguish "heal
refused by design" from "heal broken" in that payload, and neither reader did.

## The change

`rust/src/heal.rs`: the `indexed == 0` arm returns `deferred("empty_index_never_creates")`
instead of `HealOutcome::default()`. The payload now carries
`self_healed: false, heal_deferred: "empty_index_never_creates"` beside the staleness fields;
`cort usage` sees it through the same `summarize` merge as every other deferred reason (the
usage allowlist already admitted the key).

Pinned by `an_empty_index_names_the_never_create_boundary_instead_of_staying_silent`
(`rust/tests/cli.rs`), which reproduces the husk exactly: index a fixture, `DELETE FROM
file_state`, query.

## Deliberately unchanged

- **The refusal itself.** An empty index still never heals into existence; the bootstrap lever
  stays an explicit `cort index`.
- **The `Err` arm stays silent.** A `file_state` count that errors is ambiguous — a transient
  `SQLITE_IOERR_FSYNC` and a pre-`file_state` schema look identical there — and only the
  deterministic count earned a deterministic name. (An older schema is migrated by `ensure_schema`
  on open, so the missing-table case is narrower than it sounds.)
- **The green-path contract.** A fresh index and a disabled heal (`CORT_NO_HEAL=1`) still add no
  keys at all; the empty index is not the green path.
- **The skill text.** `skills/ast-grep/SKILL.md` ties `heal_deferred` to naming why a heal did
  not happen; a new reason widens the vocabulary without contradicting a word of it, so the file
  stays byte-identical.
