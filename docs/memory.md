# Project memory owned by cort

`cort memory` stores reviewed project claims for later Claude Code, Codex, and other harness
sessions. It uses the existing `project_memory(project, id, body)` Turso table and the same
normalized Git origin project key as the earlier ClaudeCat client. A correction retains the old
entry and explicitly supersedes it. Recall reports whether each cited source file still has the
same bytes; a matching hash is not proof that the claim follows from its source line.

Memory credentials, upload state, and locks live under `$CORT_MEMORY_DIR` or
`${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung/memory`. This directory holds
durable local data, separate from `CORT_CACHE_DIR`'s rebuildable code indexes. The ledger
is read and written directly in the shared database. Each machine must configure the same Turso URL and token;
`remote: ok` says this invocation read the shared database. `CORT_MEMORY_PROJECT` can override
Git origin for a project without one; the earlier `CLAUDECAT_MEMORY_PROJECT` spelling is accepted during migration.
Manual memory commands and notes upload use `CORT_COLLECT_NET_TIMEOUT_SECS` (default 120 seconds)
as their network deadline; SessionStart has its own 2.5-second dispatch deadline.
`notes upload` waits up to `CORT_MEMORY_LOCK_WAIT_SECS` (default 30 seconds, fractions allowed) for `memory.lock`; `list` and `show` read the shared database without that lock.

The shared ledger database is `harnessuse` (`turso://harnessuse-yanggf8.aws-ap-northeast-1.turso.io`,
native TYPE Turso; measurement lives in `cort-measure`); mint its token with
`turso db tokens create harnessuse`. Older credentials name `cortex-v5` on a different account;
that database carries a code-index `chunks_fts` table that blocked the old sync engine (fts5).
Use `harnessuse` for this ledger — see
`docs/2026-10-19-memory-deployment-harnessuse.md`.

## Move an existing machine

The old ClaudeCat credential file can be imported without printing its token:

If that machine has writes still pending in ClaudeCat's old replica, run the old
`claudecat memory sync` before upgrading ClaudeCat to the forwarding version.
The config import transfers credentials, not unsynced local rows.

```sh
cort memory import-config --from "$HOME/.local/share/claudecat/turso-memory.json"
cort memory check --root .
cort memory status --root .
```

If `CLAUDECAT_DATA_DIR` was customized, use its `turso-memory.json` path. The import creates
a private cort config; it does not delete ClaudeCat's files or copy remote rows.
`cort memory check` reads the shared database and reports its project id, total and active entry
counts, and conflicts. To configure a new machine, pass the token only through stdin:

```sh
cort memory configure --url <Turso URL> --token-stdin
```

`cort memory status` reports the project id, database URL, `remote: ok`, total and active entry
counts, and conflicts. A failed read returns an error with `remote: <kind>` instead.
Run it on each machine before switching agent instructions. A successful read confirms access to
the shared database; it does not establish agent answer quality.

## Harness notes

On a Claude Code private-memory save with frontmatter type `project` or `feedback`, the edit hook prompts the agent to record any project fact or correction in the ledger. This requires memory configuration but does not require notes opt-in; `MEMORY.md` is excluded.

The edit hook kicks `cort memory notes upload --background` for Claude Code memory files; a used machine also kicks it every `CORT_NOTES_UPLOAD_INTERVAL_SECS` (default 1800 seconds). `CORT_NOTES_UPLOAD_DEADLINE_SECS` bounds a background run (default 600 seconds). Hooks never wait for network work. Each pass appends a JSON line to `memory-upload.log` under `CORT_MEMORY_DIR` (or the default memory data directory).

`cort memory notes` is opt-in per project per machine. After `notes opt-in --root .`,
`notes upload` uploads Claude Code `projects/*/memory/*.md` files for opted-in projects.
Its private `notes-uploaded.json` file records each uploaded note's full key and SHA-256. A normal upload plans from this file without reading the remote notes table; if the file is missing or invalid, upload reads this machine's rows once to recover tombstones. Failed recovery is reported as `tombstones_unknown` and retried on the next upload. While the old
replica directory still exists, an older `cort` on the same machine may write these rows without
updating the file, so every upload reads this machine's rows instead (`state: rebuilt_legacy_replica`).
The file cannot tell that it was restored from an older copy: a note deleted after that copy was
made gets no tombstone. Delete `notes-uploaded.json` instead of restoring it; the next upload
rebuilds it from the remote.
The same applies after the shared database is recreated: the file still lists the old uploads, so
only edited notes would return until it is deleted.
The secret screen is coarse and fail-closed; it is not a privacy guarantee. Run
`cort memory notes upload --dry-run` to list what would be uploaded before uploading.
Claude Code's SessionStart context lists up to 15 unreviewed note names from other machines for this project, without their bodies.

## Review, record, recall

```sh
cort memory record --key 'deployment option' --claim 'Option B is current' \
  --source docs/decision.md --line 12
cort memory recall 'deployment option' --harness claude-code
```

`cort memory --help` lists every memory verb with all of its options, rendered from the parser;
`cort memory <verb> --help` (or a group such as `notes`) answers with just that part.

`record` accepts a claim without a source. When a natural source exists, pass `--source` and
`--line` together; the file must be tracked and committed, and the cited line nonblank. For a correction,
pass `--replace` for the sole active entry, or `--supersedes <active-id>` (a unique prefix of at
least 8 lowercase hex characters works). For concurrent active entries, repeat `--supersedes` for
every active id reported by `status`; `cort memory resolve` is an alias for this form.
Writes attempt to create the `project_memory_chain` unique index, which prevents two first entries
for one key, or two entries superseding the same id, within a project id. A competing write returns `memory_key_taken`
with the existing id and writes nothing; recall it and ask the user which claim holds before
superseding. Index creation is best-effort so legacy duplicate chains can still be resolved; the
index covers the whole table, so while any project in the database still holds such a duplicate
it is not created and concurrent writes fall back to the post-write `memory_conflict` check.
Conflicts can still arise from older cort builds or across project aliases because the index is
per project id. List the active entries and ask the user which holds before using `cort memory resolve`.
Recall returns active hits with `unsourced`, `source_unchanged`, `source_moved`, `source_changed`, `source_missing`, or
`conflict`, plus the cited line's status: `same_line`; `moved` with `source_line_now` when the
line's exact text appears at exactly one other line (for example because text was inserted or
removed above it; the check does not establish why);
`moved_ambiguous` when it appears at several; `changed` when it appears nowhere. For rows with
a line hash, `same_line` yields `source_unchanged`, `moved` yields `source_moved`, and
`moved_ambiguous` or `changed` yields `source_changed`. An unreadable source yields
`source_missing`, while `conflict` takes precedence. Unsourced hits print no `source`, `source_line`,
or `source_sha256`, have no source status or file-change field, and rest on their recorded time and
machine. `recorded_at_utc` is the readable UTC form of `recorded_at` when it can be formatted.
Older rows without a line hash use the
whole-file hash for their evidence status. Whenever the file is readable, `source_file_changed`
reports whether its bytes differ from the recorded file hash. Neither label checks the
surroundings: a line that moved from a "Current" section to a "Deprecated" one still reports
`source_moved`, and a line that stayed put under a heading rewritten that way still reports
`source_unchanged` (with `source_file_changed: true`). That is the 2026-10-30 trade: the label
follows the cited line because whole-file changes flagged 21 of 22 intact lines. Read the line
and its section at `source_line_now` for a moved hit, and read the section of any hit whose
claim depends on it when `source_file_changed` is true; a supersession is unnecessary only if
that context still supports the claim. SessionStart cites a moved hit at `source_line_now`. Read the cited line before using a
hit. Rows written by cort since 2026-10-20 (`63f7de3f`) carry `recorded_machine` and
`recorded_machine_source`, the writer's `usage::machine_id` and how it was derived. It is a
self-reported hint, not proof: `CORT_MACHINE_ID` overrides it, the fallbacks are a hostname
hash or `unknown`, and a machine still running an older cort writes rows with neither field,
whose origin stays unknown. An absent
ledger and a present ledger with no match are distinct. The `--harness` label is self-reported;
the memory rows are shared across harnesses, while cort's local usage row records the label and
machine without the query text.

The earlier `.claudecat/memory.json` ledger and its `import-legacy` importer were retired on
2026-10-30. (The unrelated `cort read` / `cort recall` file-reading verbs were removed on
2026-10-23; they never read this decision ledger.)

## After a repository rename

The project id is a hash of the normalized `origin` URL, so renaming the repository (or moving
it to another owner) starts an empty ledger under a new id. Link the old id instead of moving
rows:

```sh
cort memory alias add --from-remote https://github.com/<owner>/<old-name>.git --root .
cort memory alias list --root .
```

`--from-remote` hashes the URL the same way `origin` is hashed: the `git@host:owner/repo`,
`ssh://git@host/owner/repo` and `https://host/owner/repo` spellings of one repository give the same
id, but any other user or credentials part (`https://user:token@host/...`) stays in the hash, so give
the old URL as the old `origin` spelled it. When the URL is not known, pass `--from-project <id>` with the
`project_id` an earlier `cort memory status` printed. `add` refuses the current id itself and an
old id whose linked ids hold no entries at all (usually a mistyped URL); it answers
`already_linked` when the two ids are already connected, directly or through other aliases.

Aliases are read as an undirected graph: every command, and Claude Code's SessionStart recall,
reads the entries of every id connected to the current one, and `status` lists them in
`project_ids`. Harness notes are not aliased; they stay under the id each machine uploaded them
with. New entries are still written under the current id, and `--supersedes` or
`resolve` may name an id stored under a linked one. Upgrade every machine that writes this
project before adding the alias: an older cort sees only its own id, so it can still supersede
an entry a newer machine already replaced, which later shows up as a conflict to `resolve`.

`cort memory alias remove --from-remote <url> | --from-project <id>` deletes the one alias row
joining that id and the current one, whichever way it was stored; it never deletes entries. It
refuses (`alias_in_use`) when an entry's `--supersedes` or `resolve` target would end up on the
other side. If a ledger read fails with `supersedes missing id`, the usual cause is a removed
alias; adding it back repairs the read.

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
it does not establish agent answer quality. On 2026-10-22, a third machine
(`e4b1d0199c504367`) configured against `harnessuse` directly (`--token-stdin`, token minted
by the command above) and exercised both `recorded_machine` behaviors live: recall of the
2026-10-19 claim returned no machine fields (an older cort wrote it), and a freshly recorded
claim carried the writer's stamp. The same machine then wrote a reviewed correction superseding
that 2026-10-19 row — its source had gone `source_changed` — and recall returned exactly the
active claim with `source_unchanged`, the superseded entry retained in the ledger, and no
conflicts. This checks correction handling from the writer's side; the read half — a second
physical machine returning only the active claim — is still outstanding.

The same day, a replica with no local history was pulled on that same machine: an empty
`CORT_MEMORY_DIR` holding only a freshly configured connection, synchronized against
`harnessuse` (4 entries, 2 active, no conflicts), then asked for `memory ownership`. It returned
exactly one hit — the active claim, `source_unchanged`, `same_line`, superseding the retained
2026-10-19 row — field for field what the machine's long-lived replica returned. This checks
that supersession survives a from-scratch sync. It is **not** the second-machine check: reader
and writer are one physical machine, so it cannot show a stamp written elsewhere being read, nor
a reader whose cort or checkout is older than the writer's.

The local tests cover source change, supersession, schema compatibility, project isolation,
config migration, and query redaction. The read half was checked on 2026-10-30: machines
`09a907a06cdea300` and `2eb02d46ec5c4319`, both running `d4e92a6a` and neither the writer, each
recalled `memory ownership` as exactly one active hit — id `b479302767d36c88…`, written by
`e4b1d0199c504367`. Its status was `source_moved`, not the `source_unchanged` this paragraph
expected: lines inserted above it in `AGENTS.md` moved the cited line from 227 to 274, and
the section there still states the claim, so it needs no supersession. Fresh Claude Code
and Codex sessions must still be graded on the
same frozen questions, including a superseded claim, stale source, and false pasted report.
A successful remote read or a hook fire does not prove the later answer is correct. The ordered
experiment and product priorities are in `docs/2026-10-20-direction-after-collection.md`.
