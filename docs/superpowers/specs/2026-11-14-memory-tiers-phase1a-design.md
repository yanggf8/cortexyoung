# Memory tiers phase 1a: lifecycle table, review and hiding (issue #11)

Status: draft, 2026-11-14, revised twice after external review (round 1: 2 BLOCK, 11 SHOULD; round 2: 4 SHOULD). Implements
phase 1a of `2026-11-14-memory-tiers-design.md` (the parent spec; its decisions U1-U5 stand). Phase 0
evidence: ledger key `memory-tiers-decisions` and commits `90789b29..b29bd4ad`. The calls this spec
makes that the parent left open are listed under **Calls for the user to check**; everything else
follows the parent.

## What 1a delivers

1. A shared, append-only table `project_memory_lifecycle` and a pure fold over it.
2. Write verbs `cort memory promote|archive|drop|restore`, and the per-project switch
   `cort memory lifecycle on|off`.
3. A read verb, `cort memory review`.
4. Recall hides archived and dropped entries (`--all` shows them) and labels entries due for review.
5. SessionStart reads the table as one more optional statement, hides archived and dropped entries,
   puts entries due for review after the rest, and on the weekend prints one review line.

Not in 1a: `anchor` events and anchor ordering (1b), links, `use`, strength and cascade candidates
(2), `merge` (3).

## Storage

```
CREATE TABLE IF NOT EXISTS project_memory_lifecycle
  (project TEXT NOT NULL, id TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (project, id))
```

Same shape as `project_reminders`; created by the `CREATE ... IF NOT EXISTS` batch every ledger read
already sends (`Ledger::read_bodies`, whose positional `results[4]` becomes a named index the way
`alias_snapshot` does it). Written with `INSERT OR IGNORE`, one row per event, never updated or
deleted. The table joins `alias_snapshot` and the cross-component checks of `alias add`/`alias
remove` exactly as `project_reminders` does. The body is JSON:

| field | when | meaning |
|---|---|---|
| `id` | always | random event id (the reminders generator) |
| `kind` | always | `on`, `off`, `promote`, `archive`, `drop`, `restore`; any other value decodes as `unknown` (a newer build's kind: kept, changes nothing) |
| `entry` | all but `on`/`off` | the full ledger entry id the event acts on |
| `superseded_by` | `archive`, `drop` | optional: the full id of an entry that covers this one |
| `quote` | `archive`, `drop` | optional: the user's words releasing the entry (1-600 bytes, not blank) |
| `note` | any | optional free text (≤ 300 bytes) |
| `recorded_at`, `recorded_machine`, `recorded_machine_source`, `harness` | always | as reminders |

Rows are read over the alias component (`project IN (COMPONENT)`); entry ids are unique across
projects, so an event recorded under an old project id still applies.

**Decoding.** A row whose JSON or known-kind fields do not parse is skipped and counted; every other
row still applies. Every read that folds reports the count as `lifecycle_unreadable: <n>` when n > 0
(recall, review, list, the write verbs' answers), and SessionStart records it in its usage row as
`lifecycle_unreadable: <n>`. A bad row loses only the event it held -- never the whole project's
lifecycle, and no probe or repair row is written to the shared database -- but that event's loss
can cut either way: a lost `restore` leaves an entry hidden, a lost `off` leaves the project
enabled, a lost first `on` moves the epoch or leaves the project off. So whenever n > 0 the answer
also says `cort memory review --all` lists what the fold applied, and `recall --all` finds anything
hidden.

**Clock skew between machines.** A writer stamps `recorded_at = max(now, latest event on the same
entry -- or, for `on`/`off`, the latest switch event -- + 1 ns)`, from the rows it just read. A
confirmed write is then never ordered before the event it was checked against. Two machines writing
to the same entry at the same moment can still stamp the same instant; the random event id then
decides the order, and the loser's event stays inert in the history (its writer printed a success
the fold does not honour -- rare, visible in `review --all`, and undone by one more command).

## The fold (`memory_lifecycle.rs`, pure)

Input: the ledger, the decoded events, `now`. Events are ordered by `recorded_at`, then `id`.

**Switch and epoch.** The project is *enabled* when its latest `on`/`off` event is `on`. Its
**tiers epoch** is the `recorded_at` of the `on` that began its current enabled run: the first `on`
after its latest `off`, or its first `on` ever when it was never turned off (repeated `on` events
inside a run do not move it). This replaces the parent's single code constant: each project's
derived placement starts the day it is switched on, so a project enabled months later -- or
re-enabled after a long `off` -- is placed the way the parent's U2 placed agent-portal-tpm, not
dumped wholesale into review. Events written during an earlier run still apply on top of the new
placement (a promoted entry stays long-term, an archived one stays hidden). A project with no `on` event behaves exactly as
today everywhere; lifecycle write verbs other than `lifecycle on` refuse with `memory_lifecycle_off`.

**Placement before events** is phase 0's `memory_tiers::place` with that epoch: long-term with last
activity = `recorded_at`, or short-term expiring at `max(recorded_at, epoch) + 7d`.

**Entries in scope** are the active ledger entries (not superseded). Events naming a superseded
entry are kept and ignored (state `superseded` in `review --all`); events naming an id the ledger
does not hold are orphans (listed by `review --all`). `--superseded-by` is for a newer entry under a
*different* key; an old claim under the same key is retired by `record --replace`, which already
makes it inactive and needs no archive.

**Events per entry, in order.** Each event is checked against the state it lands on; one that is not
allowed there stays in the history and changes nothing:

| event | allowed when | result |
|---|---|---|
| `promote` | short-term (expired or not) | long-term, last activity = event time |
| `drop` | short-term (expired or not) | hidden, labelled `dropped` |
| `archive` | short-term or long-term | hidden, labelled `archived` |
| `restore` | `archived` or `dropped` | the tier it had before; long-term: last activity = event time; short-term: expires at event time + 7d |
| `unknown` | -- | nothing |

`drop` and `archive` differ in 1a only in which states allow them and the label they leave.

**Derived state at `now`**: `short_term`, `short_term_expired`, `long_term`,
`forgetting_candidate` (long-term with last activity + 60d ≤ now; 1a has no anchors and no links, so
age alone decides), `archived`, `dropped`. `due_at` is the expiry or last activity + 60d.
*Due for review* means `short_term_expired` or `forgetting_candidate`; *hidden* means `archived` or
`dropped`.

## Write verbs

`cort memory promote|archive|drop|restore <id> [--note T] [--lean]`, and for `archive`/`drop` also
`[--superseded-by <id> | --quote '<the user's words>']`. Each reads the ledger and the lifecycle rows
in one pipeline, folds, resolves `<id>` with the rule `--supersedes` uses (`resolve_ids`: an exact id
names itself, otherwise ≥ 8 lowercase hex as a unique prefix over all ledger entries), refuses a
superseded entry with `memory_entry_superseded` (naming its successor), checks the transition table,
writes one event, and prints the entry's new state (JSON, or one `--lean` line). Errors name the
next step: `memory_lifecycle_off`, `memory_id_unknown`, `memory_id_ambiguous`,
`memory_lifecycle_not_allowed` (current state and the verbs it allows),
`memory_archive_needs_release`, `memory_quote_invalid`. An unknown write outcome is reported as
`remind add` reports it (`outcome_unknown`) and never retried automatically.

**Archive guard, moved forward from 1b** (call C2): `archive` and `drop` of an entry that mentions
the user (`memory_tiers::mentions_user`: `user` case-insensitive, `用戶`, simplified `用户` (added during Task 4 review), `使用者`, in key or claim)
are refused with `memory_archive_needs_release` unless the command carries `--superseded-by` naming
a different, active, non-hidden entry, or a valid `--quote`. As explained to the user on 2026-11-14
(relayed by bsr): when an old fact goes stale the agent records a new sourced fact and archives the
old one with `--superseded-by`; only an entry nothing can replace needs the user's words.

`cort memory lifecycle on|off [--note T]` writes the switch event for the current project; `off`
restores today's behaviour everywhere and keeps every event for a later `on`.

## Read paths

**`cort memory review [--all] [--lean]`** (read only). On an enabled project: the entries due for
review, oldest `due_at` first, each with id, key, claim, state, `recorded_at`, `due_at`,
`mentions_user`, and the commands that apply (`promote`; `drop` or `archive`, with the guard's flag
when `mentions_user`). A header carries counts per state and the epoch. `--all` adds hidden and
superseded-with-events entries and orphans. On a disabled project it prints `lifecycle: off` and a
what-if of switching on now -- epoch = now, with every existing event applied -- which is exactly
what the epoch rule above would place, so the pilot decision can rest on data.

**`cort memory recall`.** `Ledger::read_bodies` adds the lifecycle `CREATE` and `SELECT`. On an
enabled project, hidden entries are removed before `select_terms` (they neither appear nor change
the partial fallback); `--all` keeps them. `MemoryHit` gains `lifecycle: Option<String>`, serialized
only when the state is due for review or hidden; healthy hits are byte-identical to today. The
`conflict` evidence label still counts every active entry under the key, hidden ones included (the
key is still taken for `record`), so a visible hit may say `conflict` while its partner is hidden;
`recall --all` shows both.

**`cort memory list`.** Disabled project: unchanged from phase 0. Enabled project: the tier column
becomes the derived state above (hidden entries listed with their label), the default epoch is the
project's, and `--epoch` overrides only the epoch -- events still apply -- as a what-if.

**SessionStart (`hook-recall`, and Kimi's first-prompt path that renders the same text).**
`session_recall_with` adds `SELECT body FROM project_memory_lifecycle WHERE project {scope}` as an
optional step handled like the questions step: an SQL error drops that read only, prints nothing,
and is recorded in the usage row as `lifecycle: "dropped"` (`session_usage_fields`); a dropped read,
or an answer from the unaliased retry (whose `= ?` scope could miss an `on` or `off` written under
another alias id -- the same reason `reminders_full_scope` exists), means today's behaviour for that
session. When the read succeeds over the full component and the project is enabled:
- `recall_ledger_for_hook` takes the fold. Hidden entries are removed **before** the sort and the
  20-entry cut in `recall_ledger_inner`; the sort key becomes (due for review last, then
  `recorded_at` newest first, then id), and `render_with_omitted` sorts by the same key instead of
  `recorded_at` alone, so the cut and the display agree.
- `omitted` and the "N entries left out" text count visible entries only; hidden entries are not
  mentioned.
- Entries due for review carry a ` [<state>]` suffix.
- On Saturday and Sunday, when review has at least one item, one line follows the ledger block:
  `N memory entries await review: cort memory review`. The weekday comes from the local date that
  `remind_snapshot::local_parts` returns (it gives only date and time strings, so 1a adds a small
  civil-date-to-weekday helper beside it, with its own tests); no line when `local_parts` returns
  `None`. It prints at every session start that weekend, on every machine and harness --
  deliberate: the first agent to act on it empties the count. The line's bytes (~60) are reserved
  inside the ledger's 6000-byte budget **only when the line will print** (enabled, weekend, count >
  0), so the 9,500-byte total in `hook_recall.rs` stands and every other session's cut is
  unchanged. On other days the derived demotion above is what keeps
  stale entries from reading as current rules.
No write, no lock, no extra round trip; deadline and watchdog unchanged. A disabled project's
output is byte-identical to today's.

## Old builds

They never read the table: hidden entries reappear for them; nothing breaks. Upgrading NUC,
Thinkpade15 and gpu is the usual release step.

## Tests the plan must carry

- Fold: every transition row, a not-allowed event kept inert, `restore` of a short-term entry, equal
  `recorded_at` ordered by id, `unknown` kind, orphans, events on a superseded entry, `off` after
  `on`, no switch = disabled, epoch = the `on` that began the current run (unchanged by a repeated `on`, moved by `off` then `on`; events from the earlier run still applied).
- Decoding: one undecodable row skipped and counted while the rest apply; the count reaches recall, review and the SessionStart usage row.
- Writer stamp: an existing event later than `now` makes the new event land after it.
- Placement at the boundaries (`epoch - 7d` exactly; expiry exactly at `now`).
- Guard: `user`/`用戶`/`用户`/`使用者` in key or in claim refused without a release; `--superseded-by`
  naming itself, a hidden, superseded or unknown entry refused; `--quote ''`, a blank quote and a
  601-byte quote refused with `memory_quote_invalid`.
- Prefix: exact non-hex id, ambiguous prefix, prefix of a superseded entry.
- Recall: hidden entries do not change the partial fallback; `--all`; `lifecycle` absent on healthy
  hits; a `conflict` with a hidden partner.
- Session start: lifecycle SQL error drops only that read and prints nothing (ledger, notes,
  questions, reminders intact); notes and lifecycle failing together; the unaliased retry ignores
  lifecycle; hidden entries removed before the cut (21 visible + 5 hidden shows 20 and omits 1);
  due entries after the rest and the render keeps that order; weekend line on a fixed Saturday,
  not on a fixed Friday, not when `local_parts` is `None`; disabled project byte-identical.
- Write verbs against the fake transport: one `INSERT OR IGNORE` per command; nothing written on any
  refusal or on a disabled project.
- `review` on a disabled project: `lifecycle: off` plus the epoch-now what-if with existing events applied; weekend line bytes reserved only when it prints (a full ledger on a Saturday with nothing due cuts exactly as on a Friday).

## Calls for the user to check

- **C1 Epoch per project = the `lifecycle on` that began its current run** (replaces the parent's single code constant;
  same placement for the pilot, and no wholesale review when another project is switched on later).
  For agent-portal-tpm switched on in the week of 2026-11-16, phase 0's numbers apply: the first
  short-term expiries a week later, the first forgetting candidates around 2026-12-23.
- **C2 Archive guard ships in 1a, not 1b.** Without it 1a lets an agent hide an entry quoting the
  user with no release; with it, 91% of agent-portal-tpm's entries need `--superseded-by` or
  `--quote` to be archived. Anchor confirmation stays in 1b.
- **C3 Pilot by a shared switch.** `cort memory lifecycle on` for agent-portal-tpm is one shared-DB
  write (asked through bsr first); every other project is unaffected until switched on. No project
  list is hard-coded.
- **C4 "Due for review sorts last" at session start ships in 1a, not 1b.** It is what makes an
  unreviewed weekend safe (parent U5); the rest of 1b's ordering (anchors ≤ 8 first) waits for
  anchors.
