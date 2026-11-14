# Two-tier project memory with links, decay and forgetting (issue #11) -- DRAFT for the user

Status: draft, revised once after an external design review (2 BLOCK, 6 SHOULD). Nothing is built
until the user approves it (relayed through the agent-portal-tpm NUC session, per the user's
2026-11-14 routing).

## Why

agent-portal-tpm holds 247 active ledger entries (306 rows; `cort memory status`, 2026-11-14), all
recorded since 2026-10-24, ~15 a day in November. Nothing ever leaves: 211 are unsourced, 11 cite
lines that changed or moved, and some are plainly stale (a review-routing entry still names Muse).
Session start lists the newest entries that fit 20 entries / 6000 bytes (issue #11 counts about 8 in
practice); the rest surface only through recall, where a stale entry reads like a current rule.

## The user's design (2026-11-14, issue #11) and the approved additions

1. Short-term: every new entry has an active period (7 days). When it expires the entry is reviewed:
   promote to long-term, merge, or drop.
2. Long-term: entries are linked. One with no new activity or links for too long (60 days) is
   forgotten.
3. Cascade: forgetting an entry that leaves others with no links forgets those too.
4. Links: time order (expires after a window) and promotion links.
Approved additions: anchors (rules carrying the user's words) never decay; use counts as activity;
forget = archive, restorable; a cascade goes to review, never straight to forgetting; more link types
with strength (supersedes, co-use, same topic / same source file); consolidate on promotion.

## Approaches considered

- **A. Lifecycle overlay (recommended).** Claims stay exactly as they are in `project_memory`. A new
  append-only table `project_memory_lifecycle` holds events -- `promote`, `merge`, `link`, `use`,
  `anchor`, `archive`, `restore`, `drop` -- and an entry's tier, links, liveness and archive state are
  folded from its claim row, its events and the clock at read time (events ordered by `recorded_at`
  then id, as `question.rs` folds), the way questions and reminders already fold. Older cort builds
  keep reading `project_memory` unchanged (they just do not hide archived entries), and no existing
  row is rewritten.
- B. New fields inside claim bodies: old rows would have to be rewritten or re-recorded; breaks the
  append-only ledger and confuses older builds. Rejected.
- C. Compute everything locally without storage: promotions, merges and archives are decisions that
  must be shared across machines. Rejected.

## Design (approach A)

### Tiers are derived, so migration writes nothing

An entry with no lifecycle events is placed by a rule evaluated at read time:
- recorded less than 7 days ago → short-term, expiring at `recorded_at + 7d`;
- older → long-term, last activity = its `recorded_at` (so the first forgetting reviews of the
  October entries fall due around 2026-12-23).
Migration of the existing 247 is a read rule, not a bulk write to the shared database.

### Anchors: confirmed only, guarded at archive

A claim that merely quotes or names the user is **not** treated as an anchor: in agent-portal-tpm 166
of 247 active claims contain `「` and 193 contain `用戶`, because "用戶 <date>「…」" is that project's
recording convention, and samples show both false hits (an agent's own quotes, a user's "晚點說" that
the claim itself calls undecided) and misses ("2026-10-28，用戶：ERP Architect 已決定", "用戶在 mgpu
選 A"). So:
- An **anchor** is only an entry confirmed by an `anchor` event (`cort memory anchor <id> --quote
  '<the user's words>'`). Anchors are never forgetting candidates and leave only by supersession.
- "Never lose an anchor" is enforced where loss happens -- at archive: archiving an entry whose
  claim mentions the user (`user`, `用戶`, `使用者`, case-insensitive) is refused unless the command
  carries `--superseded-by <id>` (a newer active entry covers it) or `--quote` (the user's words that
  release it). The wide rule over-asks rather than under-protects. **Open question U3.**
- Session start reserves room for short-term entries: at most 8 anchors are listed first.

### Activity and use

Activity refreshes an entry: a new claim superseding or linking to it, a `promote`/`merge`/`link`
naming it, and use. Session-start display alone is never use. What counts as use is **open question
U1**; today recall only reads (`memory_store.rs` ~1691) and a single-term query returns every entry
containing the term with no cap (`memory.rs` ~387; `recall "用戶"` returns 193), so "every recall
hit is use" would refresh most of the ledger and write without bound. The options and their costs
are listed under U1.

### Links and liveness

| link | stored or derived | strength |
|---|---|---|
| time order: recorded within 24 h of each other | derived | fades to 0 over 14 days (meaningful only for new entries) |
| promotion: chosen when a short-term entry is promoted | `link` event | starts 1.0, 60-day half-life, +0.5 per co-use |
| supersedes | existing chain | the old entry is inactive immediately |
| co-use: two entries used in one session | `use` events sharing a session id | +0.5 per occurrence, same decay |
| same source file | derived from `source` | 0.5, same 60-day half-life from the later entry's last activity |

An entry is **link-supported** while some link to a non-archived entry is above 0.25. Liveness in
phase 1 is only last activity; strength arrives in phase 2. Numbers are starting values.

### Forgetting = archive, through review only

No entry is archived by a timer. A long-term, non-anchor entry with no activity for 60 days and no
supporting link becomes a **forgetting candidate**; archiving an entry makes neighbours that lose
their last supporting link **cascade candidates**. Both are listed for review.

Until someone reviews them, the derived state still changes what agents see, with no write:
expired short-term entries and forgetting candidates sort last at session start and carry
`lifecycle: short_term_expired|forgetting_candidate` on recall hits, so a stale entry stops reading
like a current rule even if no review runs. An `archive` event hides the entry from session start
and default recall; `cort memory recall --all` still finds it, and `restore` brings it back.
Archiving does not free the key: a new claim for an archived entry's key still uses `--replace`.

### Review

`cort memory review` lists, oldest first: expired short-term entries, forgetting candidates and
cascade candidates, each with its links and the one-step commands an agent can run:
`cort memory promote <id> [--link <id>...]`, `link <a> <b>`, `archive <id> [--superseded-by <id> |
--quote '...']`, `drop <id>` (short-term only), `anchor <id> --quote '...'`, and in phase 3 `merge`.
Session start adds one line `N memory entries await review: cort memory review`. When review runs
is **open question U5**.

### Reads, writes and old builds

- SessionStart reads the lifecycle table as one more **optional** statement in the existing pipeline
  (no extra round trip), like notes/questions/reminders: an SQL error (e.g. the table does not exist
  yet on a database no one has written to) drops that read and prints one line, and archived entries
  then reappear for that session. The table joins `alias_snapshot` and the alias component.
- Rows read per session: today ~306 claim rows (~110 KB; claims average 353 bytes, max 1620) plus
  lifecycle rows. Tiering does not reduce reads -- archived rows are still read. Lifecycle rows are
  bounded by the U1 choice.
- Old builds: show archived entries until upgraded; never break.

## Phases

0. **Evidence, read-only.** (a) `cort memory list` (every active entry: key, age, source status,
   mentions-user flag). (b) `cort memory review --dry-run` on agent-portal-tpm: counts of
   short-term / long-term / expired / candidates. (c) Hand-label 30 sampled claims as anchor or not,
   to report how the archive guard would fire. (d) From local `usage.db`, the distribution of hits per
   `cort memory recall`, to size U1. (e) Baseline of SessionStart recall phase timings from `cort
   measure report`. No shared-database writes. Report goes to the user before phase 1.
1a. Lifecycle table, derived tiers, `archive`/`restore`/`promote`/`drop`, review list, recall hides
    archived and labels `lifecycle`, SessionStart optional read.
1b. `anchor` confirmation, the archive guard, session-start ordering (anchors ≤ 8, then long-term by
    last activity, then short-term; candidates last), replacing today's newest-20 cut
    (`memory.rs` ~375).
2. Links: promotion links, co-use, derived time-order and same-source links, strength, cascade
   candidates, `use` per U1.
3. Consolidation (`merge`): a merged claim supersedes parts under different keys, which today's
   `record`/`resolve` refuse (same-key only) -- phase 3 extends write validation and the chain index.
Each phase is spec → external review → small delegated tasks → gate → user approval to push.

## Open questions for the user

- **U1 What counts as use.** (a) every recall hit writes a row: unbounded writes, not feasible;
  (b) a recall whose full-term match returns ≤ 5 entries writes one row naming them, best effort after
  the answer is printed (recall becomes a remote writer); (c) rows keyed `(project, entry, day)` with
  `INSERT OR IGNORE`, so rows are capped at entries × days; (d) only an explicit `cort memory use
  <id>`: precise, but agents forget. **Recommendation: (b) + (c), plus (d) for entries found another
  way.**
- **U2 Migration.** Derived placement above (October entries long-term, first forgetting reviews
  ~12/23; stale ones demoted in display right away), or every entry older than 7 days into review now
  (~200 items)? **Recommendation: derived placement.**
- **U3 Archive guard.** Archiving an entry that mentions the user needs `--superseded-by` or the
  user's words; confirming an anchor needs the user's words; everything else an agent may do on its
  own (all reversible). Agreed?
- **U4 Merging the user's words.** When several entries are merged (phase 3), the merged claim must
  carry each part's user quote verbatim; should the user see a merge before it is recorded?
- **U5 Review cadence and load.** At ~15 entries a day about 105 short-term entries expire each week.
  Who reviews, and when -- an agent at the start of the first session in the weekend window, or
  whenever the review line appears? What happens when review falls behind is already safe (derived
  demotion), so this is about effort, not correctness.

Decided by the agent (the user may override): pilot on agent-portal-tpm for one weekly cycle before
enabling for every project (cortexyoung's own ledger included).

## What does not change

Recording, supersession (until phase 3's merge), conflicts, source checking, recall's evidence
labels; harness notes; questions and reminders.
