# Memory tiers phase 2: layered cloud search, a tree, and per-entry strength (issue #11)

Status: draft, 2026-11-15, revised twice after external review (round 1: 2 BLOCK, 5 SHOULD, 4 NIT; round 2: 4 SHOULD, 7 NIT), then reviewed by the agent that uses recall daily (2026-11-15). Replaces phase 2 of `2026-11-14-memory-tiers-design.md` (the parent
spec). The user redesigned phase 2 on 2026-11-14; their words and the agreed reading are in the
ledger under key `memory-tiers-phase2-search` (ce043cf4). The measurement behind 2a is under key
`memory-tiers-phase2-search-replay` (6b349e5e) and commit e88ffcd4. Phase 1a shipped in
`250555b1..50c4ba3a` and is enabled on agent-portal-tpm only, from 2026-11-14 23:27 Taiwan time.
Phase 1b (anchors) is independent of this spec and still to come. The calls this spec makes that
the user has not ruled on are listed under **Calls for the user to check**.

## What the user decided (2026-11-14)

1. **Layered search.** Layer 1 is the keys, which are all English, so it is searched in English.
   Layer 2 is the claims. The agent searches layer 2 only when it needs the content, querying once
   in Chinese and once in English. That is the answer to the cross-language gap; there is no
   vector search.
2. **A tree, not a graph.** "不要用圖形結構，會產生無限循環，用簡單的樹狀結構就好了". Each entry
   has at most one parent. Many-to-many relations are attribute tags, used as filters and never
   walked as edges.
3. **Time and strength are attributes of the entry**, not of links: last activity and a use
   count/score, the score decayed when it is read. Forgetting candidates and review order come
   from them.
4. **Search runs in the cloud.** "不要再增加本地資料庫破碎化了，沒事還要收拾": no local FTS cache,
   no new local database or cache file.
5. **Layer 2 is text matching on the server** ("同意用文字比對"): `json_extract` of key and claim,
   then substring matching. Three requirements: match only the key and claim fields; exclude
   superseded and archived entries inside the SQL; replaying the 109 queries returns exactly
   today's results.

This replaces the parent spec's weighted link graph (time-order, co-use and same-source links,
link strength, cascade candidates). Supersession stays the same-key version chain.

## Evidence (2026-11-14, `cort-evals recall-sql`, e88ffcd4)

- **Setup.** A byte-identical copy of agent-portal-tpm's 306 claim rows (247 active) in a
  throwaway Turso database, since destroyed.
- **What ran on the server.** SQL dropped superseded and resolved rows and narrowed by a
  substring match on `lower(key || ' ' || claim)`. All terms are AND-ed; when that matches
  nothing and there are two or more terms, they are OR-ed instead.
- **What ran on the client.** It applied today's `recall_selection` to the rows the SQL returned.
- **Result.** All 109 replayed real queries (42 CJK, 47 English, 20 identifier) gave today's ids
  in today's order, under both `LIKE ... ESCAPE` and `instr`. The SQL's active set equalled cort's
  own.
- **Rows returned.** About 21 per query on average, against the 306 every recall reads today. The
  OR stage can still return over 200 rows when a term is very common.
- **Time.** Including the network, a query took 165-300 ms at p50 and about 770 ms at worst; the
  OR stage was a second round trip.

## Phases

- **2a: cloud search.** `cort memory recall` returns only matching rows; layer 1 (`--keys`); several
  queries in one call. This is the part the evidence supports; it is specified in full below.
- **2b: tree and tags.** Parent at promotion, derived re-parenting, tag events, `--tag` filters.
  Sketched below; it gets its own detailed spec after 2a ships.
- **2c: use and strength.** Use rows per the parent spec's U1, a decayed score, and forgetting
  candidates computed from last activity and score. Sketched below, with the same rule.

Each part follows the same path: spec, external review, small delegated tasks, gate, then the
user's approval to push.

## 2a: cloud search

### The query, as SQL

One function in a new `rust/src/memory_search.rs` builds the statements. Both the product and
`cort-evals recall-sql` call it, so the replay grades what ships rather than a copy of it; the
eval's own SQL goes away. Terms are bound as parameters, never interpolated.

- **Terms.** Exactly recall's: `query.split_whitespace()`, lowercased with Rust's
  `to_lowercase`.
- **Haystack.** `lower(json_extract(body,'$.key') || ' ' || json_extract(body,'$.claim'))`, so
  only the key and claim fields are matched.
- **Match.** `instr(haystack, ?) > 0` (my call; see below).
- **Rows considered.** The alias component's rows with `json_valid(body)`. On SQLite,
  `json_extract` on malformed JSON fails the whole statement, so every statement carries this
  guard; the integrity statement below reports what it left out.
- **Active.** As `active_entries`: the body's `$.id` is not named by any row's `$.supersedes`, or by
  any element of a row's `$.resolves` (`json_each`), anywhere in the component.
- **Archived, excluded in SQL.** An entry is excluded when both hold:
  - The project is enabled: the latest switch event is `on`.
  - Of all the entry's events of kind `archive`, `drop` or `restore`, the latest is `archive`.

  This follows the 1a fold, which applies every per-entry event whatever run it was written in;
  the run only sets the epoch. Events are ordered by numeric `recorded_at`, then id. The SQL counts
  only canonical rows: valid JSON, a known kind, `recorded_at` made of digits only, and a non-empty
  `entry` on per-entry kinds.

  An entry the SQL excludes is always one the fold hides. `archive` is allowed from every visible
  state, and an `archive` on an entry already hidden leaves it hidden. The rest of the hiding is
  done by the client: `drop`, and every case the SQL does not express, are hidden by the 1a fold
  run on what came back.

  The SQL's reading of a lifecycle row can still differ from 1a's `classify`:
  - `parse_recorded_ns` accepts a leading `+` or `-` and rejects digits out of the date range.
  - A digit string beyond i64 saturates SQLite's `CAST`.
  - serde rejects a wrong field type such as `"note": 5`.
  - A duplicated JSON key is read first-wins by SQLite, while serde rejects the row, so the fold
    never applies it.

  So the SQL's decision is checked, not trusted. One more statement in the same pipeline returns
  the ids and bodies of the entries the archived condition excluded; archived entries are few. The
  client folds those entries with the lifecycle rows it already has. If the fold shows any of them,
  the client discards the answer and sends the query again without the archived condition, in one
  more round trip. Only a ledger carrying a row cort itself never writes can trigger that.

  `--all` drops the archived condition.
- **Stage 1.** Active, not archived, every term matches. This is today's full-term match.
- **Stage 2.** Sent in the same pipeline, so there is no second round trip. It returns rows only
  when stage 1 matched nothing and there are two or more terms; the SQL repeats stage 1's
  condition in a `NOT EXISTS` to know that. It uses the partial rule's own term list, taken from
  one helper in `memory.rs` that `select_terms` also calls:
  - distinct terms only;
  - noise words dropped (`PARTIAL_IGNORED`);
  - lone ASCII characters dropped.

  A row passes when `(match t1) + (match t2) + ... >= ceil(n / 2)`, which is the client's "at
  least half" threshold. It uses substring matching, not the word-start rule for ASCII terms. That
  makes it a superset of what the client keeps, and tighter than the OR prefilter that was
  measured (expect fewer than the 200+ rows a common term pulled).
  Stage 2 is not sent when that list is empty (n = 0), because today's partial rule then selects
  nothing, and a threshold of 0 would return every row.
- **Client decision.** `recall_selection` runs unchanged on the union of the returned stage-1
  and stage-2 rows, deduplicated by id. It decides the selection, the order, and the cap of 5 on
  partial matches. The union contains every entry the client's full-term match would select,
  because SQL stage 1 is a superset of it. When that match is empty, the union contains every row
  the partial rule could keep, because stage 2 is a superset of those.
- **Same id under two aliases.** Rows are deduplicated by id with `merge_bodies`' rule before
  anything else, keeping the body with more machine-stamp fields.
- **When the SQL's stage 1 and the client's disagree.** The SQL can find a stage-1 match where the
  client, after the fold and the exact full-term check, finds none. That happens in two ways:
  - every stage-1 row is hidden by the fold (a dropped entry, say);
  - a term contributed no SQL condition (see case folding).

  Today's recall would then fall through to the partial rule. Two rules cover this:
  - When any term contributed no condition, stage 2 is sent without its `NOT EXISTS` gate, in the
    same pipeline.
  - The client sends a second round trip only when all of these hold:
    - stage 1 returned rows;
    - stage 2 was sent gated;
    - the client's full-term match over the stage-1 rows is empty;
    - there are two or more terms.

    A query that matches nothing at all therefore costs no extra trip.

  There is one form of second round trip, used both here and for the archived check: the whole
  pipeline again, without the archived condition and with every stage 2 ungated. It can only
  return more rows than either narrower re-send would, and the client still decides, so a recall
  makes at most two round trips.

  Tests pin both: `café 部署`, and a full match that is only on a dropped entry.

**Case folding.** SQLite's `lower()` folds only ASCII; recall folds Unicode. A term containing a
cased non-ASCII letter (one whose upper and lower forms differ: Latin accents, Greek, full-width
Latin) contributes no condition in SQL: it is treated as matching in stage 1 and counted as matched
in stage 2. That keeps the SQL a superset of what the client matches, and the client still
decides. A term containing a combining mark (U+0300-U+036F) is exempt the same way, because
`İstanbul` lowercases to `i`, U+0307, `stanbul`, which the SQL haystack's `İ` never contains. CJK
has no case, so CJK terms keep their condition. One gap remains, on the haystack side:
a non-ASCII character that Rust lowercases to ASCII can make an ASCII term match in today's recall
and miss in SQL. Two such characters exist: the Kelvin sign K (U+212A), which lowercases to `k`,
and İ (U+0130), which lowercases to `i` followed by U+0307. None of the 247 entries has either.
The replay gate below would show it.

### Whole-ledger facts recall still needs

Today recall reads every row, so these come for free. In 2a each is one more statement in the same
pipeline. All statements of one round trip run inside one read transaction (`BEGIN` … `COMMIT` in the same
request), so they see a single state; a pipeline would run each on its own, and a write landing
between two of them could produce an answer no state ever had. A second round trip's answer
replaces the first's entirely. Each returns counts or ids, except the integrity screen and the archived-exclusion check, which return the few bodies the client must decode itself.

| Fact | Statement | Behaviour |
|---|---|---|
| `ledger_status: present` | `count(DISTINCT $.id)` over the component | as today |
| `evidence_status: conflict` | keys with two or more distinct active ids, hidden ones included (as 1a counts) | as today, for the returned hits |
| Integrity screen | the bodies of suspect rows (below) | any row sends recall to today's whole-ledger read, which fails with today's message |
| Lifecycle rows | every lifecycle row of the component (few now; 2c keeps use rows out of this table) | the 1a fold runs on the returned entries; unreadable rows are counted as in 1a |

**The integrity screen.** Today `ledger_from_bodies` runs `validate()` on the whole ledger, and
recall fails on any of eight errors. In 2a a statement returns the bodies of rows the SQL can
recognise as suspect:
- not valid JSON;
- a missing `id`, `key` or `claim`, or one that is empty after trimming ASCII whitespace;
- a predecessor that is not in the component: `supersedes`, or any element of `resolves` (via
  `json_each`), naming a missing id;
- a predecessor under a different key, or naming the entry's own id;
- one id appearing in two or more rows whose bodies are not byte-equal. The whole read the screen sends recall to
  runs today's `merge_bodies` on those rows, so `same_claim`'s tolerance of missing machine stamps is applied by
  the same code.

The client does not fetch the rows a suspect names. Any suspect row, in either search trip, sends
recall to today's whole-ledger read, one more round trip, which decodes every row and reports the
first error with today's message, serde's own text included; a suspect that passes today's checks
(the screen is not a verdict) is answered as today. With one fault the message is the one today's
read gives. With two or more, it is one of the messages today's read could give, not necessarily
the same one: today's read has no row order (its `SELECT` has no `ORDER BY`) and checks collisions
row by row as it decodes (Task 4 external review, 2026-11-16).

2026-11-16 replay: TursoDB answered 502 to the screen's second arm (deep CTE reuse); the screen
returns suspect rows only.

The checks that need serde's or `validate()`'s full reading of every row do not stay on recall:
- a well-formed JSON body that serde still rejects (a wrong field type, a negative
  `source_line`);
- a key or claim made only of non-ASCII whitespace;
- an invalid source or resolution;
- a superseded id repeated;
- a supersession cycle of two or more entries.

A cycle's entries are all named as predecessors, so the active rule drops them from recall
silently instead of failing. All of these are checked only on the rows recall returns. Over the
whole ledger they move to `cort memory status` and `list`, which still read every row and fail as
today.

**SessionStart says so (agent review, 2026-11-15).** An entry missing silently from recall is worse
for an agent than an error, because the agent takes the absence as an answer. SessionStart still
reads the whole ledger and still fails on every one of the eight errors, so every session already
sees a line. Today that line is wrong: `session_recall_with` returns `ErrorKind::Sql` for a ledger
that does not decode, and `main.rs` reports it as
`the shared database could not be read at session start (remote_unreachable)`. 2a gives this case
its own outcome, `ledger_invalid`, and its own line, naming the error and the command that shows
it:

`cort memory: the shared ledger has an integrity problem (<today's message>); run \`cort memory status\` to see it, and do not trust recall's silence until it is fixed`

The unaliased retry reads `project = ?` alone, so it decodes with the subset rule: a predecessor under an alias is legitimately absent there (whole-branch review, 2026-11-16). The usage row records the outcome as `ledger_invalid`.

The returned subset is decoded with one rule: a predecessor (`supersedes` or `resolves`) that was
not returned is not an error, because the screen already checked that it exists. Without that,
every hit whose predecessor did not match would read as "supersedes missing id". The rule touches
three places that assume the whole ledger is present:
- `ledger_from_bodies`' all-ids check;
- its topological ordering, which otherwise reports "missing id or cycle";
- `validate()`'s `by_id` lookup.

**The fold on a subset.** For every hit, 1a's fold run over the returned rows must give the same
state as the fold over the whole ledger. A test pins this with a ledger whose events name entries
that do not match the query.

### CLI

- **`cort memory recall "<q>"`** keeps today's output exactly: the same hits, order and fields.
  Only its reads change.
- **`cort memory recall "<q1>" --also "<q2>"`** (repeatable) is layer 2 with several queries; the
  intended use is once in Chinese and once in English. The extra queries take a flag on purpose.
  Today an unquoted `recall deploy plan` is refused by clap. If a second positional argument meant
  a second query, that same typo would silently become the union of two one-word queries. With
  `--also`, the positional argument stays exactly one query.

  Each query runs its own stages and selection, all in one pipeline. Hits are each query's hits in
  its own order, the queries taken in argument order, deduplicated by id with the first
  occurrence kept. The top-level `match_kind` is replaced by
  `queries: [{query, match_kind, hits: [{id, matched_terms?}]}]`. Each top-level hit gains
  `matched_queries: [index, ...]` and carries no `matched_terms` of its own. A per-query answer
  can then never be read as another query's. All of this appears only when `--also` is given, so a
  single query stays byte-identical.
- **`cort memory recall --keys "<q>"`** (with `--also` and `--all` allowed) is layer 1. It adds
  `--lean`, a new flag that layer 1 alone accepts; recall today has only `--json`.
  It uses the same stages and the same decision. `select_terms` gains a haystack parameter, key
  alone here and key plus claim everywhere else, so the decision is still a single function; the
  stage-2 superset test runs for both haystacks.

  Each hit is one compact row: `key`, `id`, `recorded_at_utc`, the lifecycle state when the
  project is enabled, and `conflict: true` when the key holds two or more active entries. There
  are no claims and no source checks; `--lean` prints one line per hit. The output names the next
  step: `cort memory recall "<words>" --also "<words in the other language>"` for the content.
- **SessionStart, `list`, `review`, `status` and the write verbs** still read the whole ledger.
  SessionStart changes only its integrity-error line (above).

### Tests and the gate

- **Replay.** `cort-evals recall-sql` now drives the product's query builder and keeps one job:
  comparing the replayed queries with the baseline over every active row. On the agent-portal-tpm
  copy that means 109 of 109 equal. The comparison is re-run on a fresh throwaway database before
  2a ships (it needs the user's approval, as this one did), and the result is recorded under
  `memory-tiers-phase2-search-replay`.
- **Unit tests, on `SqliteTransport`.** Each pins one behaviour:
  - the archived exclusion against the 1a fold:
    - archive, and archive after restore;
    - a drop the fold ignores;
    - an archive written in an earlier run, which stays excluded;
    - a project switched off;
    - an excluded entry that the fold shows (a `+`-prefixed `recorded_at` on its `restore`, an
      `archive` row with `"note": 5`, a duplicated `kind` key), each sending the query again without
      the condition, in at most two round trips;
  - the stage-2 threshold, and its superset property against the word-start rule, for both
    haystacks;
  - `café 部署`, and a full match that is only on a dropped entry;
  - one id under two aliases, with and without stamps: one hit, and no false conflict;
  - each screened integrity error giving today's message, and a hit whose predecessor did not
    match not being an error;
  - conflict across a hidden partner;
  - the fold on the subset equal to the fold on the whole ledger;
  - `--also`, and a single query staying byte-identical;
  - `--keys` output.

## 2b: tree and tags (sketch)

- **A parent at promotion.**
  - Syntax: `cort memory promote <id> --parent <id>`, or `cort memory parent <id> <parent-id>`
    later, as a `parent` event in the lifecycle table. The latest `parent` event wins, and
    `--none` clears it.
  - The parent must be active, visible and long-term.
  - Cycles are refused at write time by walking up the ancestors, with a depth cap of 16. The
    fold also refuses to apply a `parent` event that would close a cycle (two machines writing at
    once), so a read never loops.
- **Archiving a parent.** Re-parenting is derived at read time, never written: a child's effective
  parent is its nearest visible ancestor. A parent superseded by `--replace` is first followed
  along the supersession chain to its active successor, the same walk 1a's `act` makes. Children
  left with no ancestor are listed in `review` as orphans. Forgetting never cascades.
- **Tags.**
  - Written as `tag` events: `cort memory tag <id> <name>=<value>`, with `--remove`.
  - Derived tags cost no row: `source` (the cited file), `machine` (`recorded_machine`) and
    `harness`.
  - `recall --tag name=value` (repeatable) adds SQL conditions to both layers. Tags filter results;
    they are never walked.
- **Output.**
  - Layer 1 shows each hit's ancestor key path (`parent-key > key`).
  - Layer 2 shows a hit's `parent` key and number of visible children.
  - SessionStart is unchanged.

## 2c: use and strength (sketch)

- **Use rows.** These follow the parent spec's U1: (b) + (c), plus (d).
  - A recall whose full-term match returns at most 5 entries writes one `use` row per entry per
    day. The write is best effort, after the answer is printed.
  - `cort memory use <id>` covers entries found another way.
  - Rows go into their own table, `project_memory_use (project, entry, day)`, with
    `PRIMARY KEY (project, entry, day)`, the UTC date, and `INSERT OR IGNORE`. That is the parent
    spec's U1 (c). Their number is capped at entries × days, and they stay out of the lifecycle
    table that SessionStart and recall read whole.
  - SessionStart never reads the use table. `review`, `list` and the forgetting rule do.
  - Recall becomes a remote writer here, and only here.
- **Score.** `score = Σ over use days of 2^(-age_days / 60)`, computed at read time from the
  entry's own use rows. Nothing flows along tree edges: the tree carries only parent and child,
  as the user decided. Last activity is 1a's, extended by use.
- **Forgetting candidate.** A long-term, non-anchor entry becomes one when it has had no activity
  for 60 days and its score is below 0.25. These numbers are starting values, as in the parent
  spec.
- **Order.** Review lists candidates by score, lowest first. SessionStart keeps 1a's order.

## Calls for the user to check

The agent that uses recall daily (the NUC bsr session) reviewed these on 2026-11-15 and accepted
all seven, the fifth with the condition noted there.

1. **`instr` rather than `LIKE`.** Both measured equal on all 109 queries. `instr` needs no
   escaping of `%`, `_` and `\`, which are common in identifiers. The user's words were "文字比對",
   and both are text matching.
2. **"Archived" in SQL means the `archive` event.** `drop` and the remaining fold cases are hidden
   by the client, using the 1a fold on the returned rows. Expressing the whole fold in SQL would
   put a second copy of that decision on the server. The project rule is "a parser may be plural;
   the decision may not".
3. **Default recall stays layer 2**, with today's key-and-claim results. Layer 1 is `--keys`, so
   the replay requirement holds and nothing an agent does today changes.
4. **2b and 2c are sketched, not specified.** Each gets its own detailed spec after 2a ships and
   has been measured.
5. **Some integrity errors move from recall to `status` and `list`.** Recall still fails, with
   today's message, on every error the SQL screen can recognise. The rest are checked over the
   whole ledger only by `status` and `list`, which still read every row:
   - a JSON body serde rejects;
   - a key or claim made only of non-ASCII whitespace;
   - an invalid source or resolution;
   - a repeated superseded id;
   - a cycle of two or more entries, whose entries then drop out of recall silently instead of
     failing it.

   Keeping them all on recall would mean reading every row on every recall, which is what 2a
   removes. The agent review accepted this on one condition, now in the spec: SessionStart, which
   still reads every row, names the integrity error and points at `cort memory status`.
6. **Multiple queries take `--also`, not more positional arguments.** Today an unquoted
   `recall deploy plan` is an error. Under positional arguments it would silently become a union
   of two one-word queries.
7. **2c's score ignores the tree.** An earlier draft added strength for active children. That
   would make strength flow along a tree edge, against "樹只帶 parent/child、不帶權重", so it was
   removed.
