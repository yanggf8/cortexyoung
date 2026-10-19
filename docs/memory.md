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

`record` requires a tracked, committed source file and a nonblank cited line. For a correction,
pass `--supersedes <active-id>`; for two or more concurrent active entries of one key, use
`cort memory resolve` and repeat `--supersedes` for every active id reported by `status`.
Recall returns active hits with `source_unchanged`, `source_changed`, `source_missing`, or
`conflict`, plus the cited line's status. Read the cited line before using a hit. An absent
ledger and a present ledger with no match are distinct. The `--harness` label is self-reported;
the memory rows are shared across harnesses, while cort's local usage row records the label and
machine without the query text.

The earlier `.claudecat/memory.json` ledger is an explicit import source through
`cort memory import-legacy`. It is not the active database. `cort read` and `cort recall`
remain source fragment tools and do not read this decision ledger.

## Evidence still needed

On 2026-10-19, a temporary cort replica using the existing ClaudeCat Turso connection
successfully synchronized. Cortexyoung's Git-origin project had no entries; ClaudeCat's
Git-origin project exposed its existing two rows, with one active. The temporary config and
replica were removed afterward. This checks remote schema and project-key compatibility on
one machine, not a two-machine write/read round trip.

The local tests cover source change, supersession, schema compatibility, project isolation,
config migration, and query redaction. A deployment check must configure two independent
machine replicas against the same remote table, write a reviewed correction on one, and
verify that the other reads the active claim and its source status. Then fresh Claude Code and
Codex sessions should be graded on the same frozen questions, including a superseded claim,
stale source, and false pasted report. A sync succeeds or a hook fires without proving the
later answer is correct.
