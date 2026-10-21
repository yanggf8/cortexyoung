# Project memory owned by cort

`cort memory` stores reviewed project claims for later Claude Code, Codex, and other harness
sessions. It uses the existing `project_memory(project, id, body)` Turso table and the same
normalized Git origin project key as the earlier ClaudeCat client. A correction retains the old
entry and explicitly supersedes it. Recall reports whether each cited source file still has the
same bytes; a matching hash is not proof that the claim follows from its source line.

The memory connection, local sync replica, and lock now live under
`$CORT_MEMORY_DIR` or `${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung/memory`. This directory
is durable data, separate from `CORT_CACHE_DIR`'s rebuildable code indexes. The remote table
remains shared across machines. Each machine must configure the same Turso URL and token;
`sync_status: synced` says this invocation pushed and pulled, not that no other machine can
write afterward. `CORT_MEMORY_PROJECT` can override Git origin for a project without one;
the earlier `CLAUDECAT_MEMORY_PROJECT` spelling is accepted during migration.

The shared ledger database is `harnessuse` (`turso://harnessuse-yanggf8.aws-ap-northeast-1.turso.io`,
native TYPE Turso, shared with `telemetry_events`); mint its token with
`turso db tokens create harnessuse`. Older credentials name `cortex-v5` on a different account;
that database carries a code-index `chunks_fts` table the sync engine cannot open (fts5), so
configuring against it fails before any network work — see
`docs/2026-10-19-memory-deployment-harnessuse.md`.

## Move an existing machine

The old ClaudeCat credential file can be imported without printing its token:

If that machine has writes still pending in ClaudeCat's old replica, run the old
`claudecat memory sync` before upgrading ClaudeCat to the forwarding version.
The config import transfers credentials, not unsynced local rows.

```sh
cort memory import-config --from "$HOME/.local/share/claudecat/turso-memory.json"
cort memory sync --root .
cort memory status --root .
```

If `CLAUDECAT_DATA_DIR` was customized, use its `turso-memory.json` path. The import creates
a private cort config and a fresh cort replica; it does not delete ClaudeCat's files or copy
the remote rows into another table. `sync` pulls the same remote ledger into the cort-owned
replica. To configure a new machine, pass the token only through stdin:

```sh
cort memory configure --url <Turso URL> --token-stdin
```

`cort memory status` reports the project id, sync status, number of entries, and conflicts.
Run it on each machine before switching agent instructions. No cross-machine behavioral result
is inferred from a successful sync alone.

## Review, record, recall

```sh
cort memory record --key 'deployment option' --claim 'Option B is current' \
  --source docs/decision.md --line 12
cort memory recall 'deployment option' --harness claude-code
```

`cort memory --help` lists every memory verb with all of its options, rendered from the parser.

`record` requires a tracked, committed source file and a nonblank cited line. For a correction,
pass `--supersedes <active-id>`; for two or more concurrent active entries of one key, use
`cort memory resolve` and repeat `--supersedes` for every active id reported by `status`.
Recall returns active hits with `source_unchanged`, `source_changed`, `source_missing`, or
`conflict`, plus the cited line's status: `same_line`; `moved` with `source_line_now` when the
line's exact text appears at exactly one other line (for example because text was inserted or
removed above it; the check does not establish why);
`moved_ambiguous` when it appears at several; `changed` when it appears nowhere. A `moved` hit
keeps `source_changed` because the file bytes did change. `moved` locates the text; it does not
check its surroundings, so a line that moved from a "Current" section to a "Deprecated" one
still reports `moved`. Read the line and its section at `source_line_now`; a supersession is
unnecessary only if that context still supports the claim. Read the cited line before using a
hit. Rows written by cort since 2026-10-20 (`63f7de3f`) carry `recorded_machine` and
`recorded_machine_source`, the writer's `usage::machine_id` and how it was derived. It is a
self-reported hint, not proof: `CORT_MACHINE_ID` overrides it, the fallbacks are a hostname
hash or `unknown`, and a machine still running an older cort writes rows with neither field,
whose origin stays unknown. An absent
ledger and a present ledger with no match are distinct. The `--harness` label is self-reported;
the memory rows are shared across harnesses, while cort's local usage row records the label and
machine without the query text.

The earlier `.claudecat/memory.json` ledger is an explicit import source through
`cort memory import-legacy`. It is not the active database. `cort read` and `cort recall`
remain source fragment tools and do not read this decision ledger.

## Deployment evidence and the next gate

On 2026-10-19, a temporary cort replica using the existing ClaudeCat Turso connection
successfully synchronized. Cortexyoung's Git-origin project had no entries; ClaudeCat's
Git-origin project exposed its existing two rows, with one active. The temporary config and
replica were removed afterward. This checked remote schema and project-key compatibility.
Later that day, a reviewed `memory ownership` claim was recorded from `AGENTS.md` on machine
`2eb02d46ec5c4319`. On 2026-10-20, machine `44a8a38f0b05b9a8` imported its already-synced
ClaudeCat credentials into cort, synchronized its fresh cort replica against `harnessuse`, and
recalled that claim as active with `source_unchanged` and `source_line_status: same_line`. The
cited source line was read in this checkout. This checks one write-to-other-machine-read path;
it does not establish cross-machine correction handling or agent answer quality.

The local tests cover source change, supersession, schema compatibility, project isolation,
config migration, and query redaction. The remaining deployment check is to write a reviewed
correction on one physical machine and verify that the other reads only the active claim with
the expected source status. Fresh Claude Code and Codex sessions must then be graded on the
same frozen questions, including a superseded claim, stale source, and false pasted report.
A sync succeeds or a hook fires without proving the later answer is correct. The ordered
experiment and product priorities are in `docs/2026-10-20-direction-after-collection.md`.
