# Memory deployment on one machine: two false starts and the right door, 2026-10-19

Machine `2eb02d46ec5c4319` (`etc-machine-id`), repo head `3fed89ed`, cort 0.1.0 built from that
head, Turso CLI v1.0.34. This records one machine's configure → sync → record → recall round
trip. The two-machine write/read round trip demanded by `docs/memory.md` ("Evidence still
needed") was still open at the time of this deployment.

## Second-machine read, 2026-10-20

Machine `44a8a38f0b05b9a8` upgraded cort and ClaudeCat, synchronized the old ClaudeCat
replica first, imported its `harnessuse` credentials into cort, and synchronized a fresh
cort-owned replica. `cort memory status` reported one active cortexyoung entry and no
conflicts. `cort memory recall 'memory ownership' --harness codex` returned the entry
recorded below, with `sync_status: synced`, `evidence_status: source_unchanged`, and
`source_line_status: same_line`; the cited `AGENTS.md:227` context was read in that checkout.
ClaudeCat's forwarding `memory recall` returned the same entry. This closes the single
write-to-other-machine-read check, not the correction round trip or a paired test of agent
answers. See `docs/memory.md` for the remaining gate.

## First failure: the code arrived, the tool did not

`git pull` landed `3fed89ed` (which adds `cort memory`) at 10:47 and an agent was asked to use
it. It could not, and the working tree was clean: the failure left no diff to inspect. The
installed `cort` was the 2026-09-29 deployment (shim → `cort-953c43ed3fa4`); its subcommand list
has no `memory`. The source itself compiled cleanly (`cargo check --all-targets --locked`, 1m16s,
including the new `turso 0.9.0-pre.1`), so the gap was only between the tree and
`~/.cargo/bin/cort`. The fix is `rust/target/release/cort_upgrade`: `install.sh` refuses to
install over an existing one and says so ("run this tree's cort_upgrade"), which is correct
behavior — `--check` diagnosed binary, usage.db schema 1→2, and one drifted index; the real run
activated all three. The general point is the one the claudecat audit hit on 2026-09-18: a pull
updates the tree, never the installed tool, and the frozen version number distinguishes neither —
the subcommand surface does.

## Second failure: fts5, not auth

`cort memory configure` with the credentials sitting in `~/.secrets` succeeded; `sync` then died
opening the local replica:

    cannot open Turso sync replica: ... unable to open database file:
    Extension error: Virtual table module not found: fts5

Nothing in `memory_store.rs` creates fts5 — the crate's own fts5 tables belong to the code index
(`rust/src/schema.sql:98`). `strings` over the freshly created replica's `memory.db` showed the
guilty schema: `CREATE VIRTUAL TABLE chunks_fts USING fts5(` — a code-index table, replicated
down from the remote database. The sync engine (`turso_core`, the pure-Rust engine) does not ship
fts5 and cannot open a schema that contains it.

## The wrong door

`TURSO_DATABASE_URL` named `libsql://cortex-v5-lisyanggf.aws-eu-west-1.turso.io`. The account
logged in on this machine is `yanggf8` (single org `personal`), so that database is not even
visible from here; the JWT's shape (267 bytes, two dot-separated segments) was never the problem,
and neither was OAuth versus minted tokens — the failure was local, before any network work. An
old credential for an old experiment's database is a key that opens a door, just not one this
product should walk through.

## The right door: harnessuse

`turso db shell harnessuse` over `sqlite_master` lists `project_memory`, `telemetry_events`, and
`turso_sync_last_change_id` — the ledger, the telemetry table, and the sync bookkeeping, in one
native TYPE-Turso database. Configure piped `turso db tokens create harnessuse` into
`--token-stdin`; sync answered `synced 0 project memory entries` (consistent with the 2026-10-18
probe: this project's ledger was empty) and `status` reports `project_id` from git origin with
`sync_status: synced`. The first entry was recorded from `AGENTS.md:227` (the 2026-10-19
ownership correction), pushed, and recalled with `evidence_status: source_unchanged` and
`source_line_status: same_line`, machine-stamped. `docs/memory.md` now names the database so the
next machine does not repeat this walk.

## Machine-local notes

- The Turso CLI updates through `turso update`; there is no `upgrade` subcommand in v1.0.32.
- `turso auth login` under WSL opens the OAuth page through `xdg-open`, which fails silently
  there; the URL the terminal prints must be opened in the Windows browser by hand.
- The stale `TURSO_*` lines in `~/.secrets` are commented out on this machine only. Any machine
  still holding a cortex-v5 credential will reproduce the fts5 failure until it reconfigures
  against harnessuse.
