# codegraph lessons: a three-item proposal, for review

Status: item B **landed** at `d6ea1991` (2026-09-17), all five implementation-review findings
applied; the standing census on this repo at that commit is 34 of 9,971 raw receiver edges
attached — 9,586 zero-candidate, 185 multi-candidate, 145 ownerless, 21 binding-refused, none
shape-refused. Item D **landed** at `ced8795f` (2026-09-17) — the stamp rides the *row*, not the
sidecar, for the reason the section now records. Item A **landed** at `2b099e12` (2026-09-17) as
`gate-audit --report FILE`: the first generated table (venue `.` at `ced8795f`, machine
`2eb02d46ec5c4319/etc-machine-id`) reads population 10,030, attached 34, refused 9,996 — 9,643
zero-candidate, 186 multi-candidate, 146 ownerless, 21 binding-refused, 0 shape-refused — same
shape as the `d6ea1991` census, moved by tree growth, the binding-refused population holding at
21. Item C is dropped by its gate (below). Originally
written 2026-09-16 after reading
`https://github.com/colbymchenry/codegraph` (README at `main`; 71.1k stars, MIT; Rust parsing
kernel + SQLite WAL + FTS5 + OS-watcher sync + MCP surface). Reviewed by Codex the same day
(`codex exec --sandbox read-only`, 156k tokens); every "corrections from review" block below is
that review applied — the reviewer verified the plan's claims about current code and found three
material errors, marked where they were. The review was then **independently re-verified claim by
claim against the sources** the same day: every code claim checked out (`stamp_machine`,
`run_status_json`'s missing stamp, `venue_head`'s refusal of non-git, recall.rs's canonicalized
absolute venue, `ReceiverIndex` keying all chunks by last segment including trait methods,
`receiver_binds`' attach conditions, the stream fold's final-result-only usage, the
`arms.rs`/`summary.rs`/`harness.rs` mechanics, the dropped receiver-call file attribution). Two
things the re-verification found that the review missed are folded in below: the Item A placement
conflicts with `recall.rs`'s stated charter, and Item C's key stream-shape assumption is
unverifiable from committed evidence and is now a feasibility gate. **That gate ran the same day
and DROPPED item C** (see the verdict inside; the machine's routed backend reports zeroed
per-message usage, and the pre-registered rule says dropped, not approximated). Item D (run-row
machine identity) is the one piece of C that survives. A second Codex pass on Item B (2026-09-17,
against the committed doc and current HEAD) returned CHANGE — the four-class core reaffirmed, the
trait/impl tag dropped as underivable from the index, and the sibling-module placement plus a
`no-receiver-shape` sub-count folded into Items A and B; every new claim was re-verified against
`src/pack/rules/rust.yml` and `rust/src/schema.sql` before being applied. A third pass reviewed
the Item B implementation itself (2026-09-17): CHANGE — the shape precondition became a shared
`graph.rs` primitive instead of a copied predicate, the class renamed `binding_refused` because
shape precedes ownership in the gate's order, the index connection made truly read-only
(`open_db` writes), and the venue reported under the spelling as passed; all five findings were
verified against source before being applied. This doc states what we
propose to learn,
what we verified we already do as well or better, and what we reject. Nothing here changes
product behavior: items A, B and D live in the `evals` crate and are measurement, presentation
and provenance.

The product centers differ. codegraph answers comprehension and flow questions ("how does X reach
Y", dynamic-dispatch hops, framework route edges); `cort` answers caller-set questions whose edges
can be checked one by one and whose completeness is disclosed. Every item below is judged by the
repo rule: **a change that makes an answer cheaper to verify is on the main line; a feature that
only makes answers more numerous is not.**

## What we verified we already do as well or better (context, not work)

1. **Control-arm contamination.** Their fix (CLI blocked in both arms via sanitized PATH +
   `PreToolUse` deny, after measuring 26-of-28 unblocked control runs reaching the tool through
   Bash) parallels ours, found independently: audit F-11 (headless `--allowedTools` does not bind
   Bash), the PATH jail of per-arm whitelisted binaries (`evals/src/arms.rs`), `arm_held` on every
   row, and the README's refusal to average unheld cells. No work.
2. **Edge-level verification.** Their "fair coverage" is *file-level* (share of symbol-bearing
   files with >= 1 resolved cross-file dependent): a missed second caller inside a covered file is
   invisible to it, so their 100%-on-requests cannot mean what our numbers mean. Their
   "byte-for-byte identical to the reference engine" is their own two engines agreeing —
   determinism, not correctness. `verify-impact` grades an edge against its one call site. No work.
3. **Demand grounding.** Their README carries no demand analysis; their seven benchmark queries
   are all comprehension questions. The corpus (1,214 instructions,
   `docs/2026-08-31-demand-recheck.md`) stands. No work.
4. **Cost honesty.** We already undercut our own 7.7x payload headline with the 2.8x money figure
   (README, "the payload ratio is not the money"). Their residual-context admission is the same
   discipline applied to an axis they lose; item C borrows the *axis*, not their metric (see the
   correction there).

## Item A — a standing, regenerable measured-coverage report

**Landed at `2b099e12` (2026-09-17).** `--report FILE` writes the markdown table rendered from
the same report Value the JSON stdout prints — one measured value, two renderings; a report-shape
change lands as `absent` in the golden snapshot rather than as a second computation that could
quietly disagree. Both heads print on every row (a superset of "index head when it differs"),
`heads_agree` beside them, machine id/source stamped before rendering, the venue under the
spelling as passed. The file is written before anything prints and storage failures are
returned, so a full disk is the error rather than something behind JSON the caller trusted.
`recall-exp` is untouched. No artifact is checked in yet: a committed table stays a deliberate
act, regenerated at whatever head is being quoted.

**Problem.** The receiver-gate recall numbers (9 of 4,833 receiver call sites attached at
`a0269cda`; 12 of 5,843 at `dbc971f7`) live in CLAUDE.md and docs prose, quoted per commit, with
no single artifact that regenerates them. codegraph's README coverage table is their trust
center; the *presentation* is worth copying and the *provenance* is not: their table carries no
commit, no machine stamp, and no reproduction command, so by this repo's own rule none of their
numbers are quotable.

**Population — corrected after review.** The first draft defined the table over the textual
population `recall.rs` already scans, minus relationship rows. That is not a valid partition:
`recall-exp` deliberately reads source only and says attached counts must come from the index
(`evals/src/recall.rs` module doc); the two sides have different universes, and relationships
collapse repeated source->target edges. The population is instead the *indexed* one:
`raw_edges WHERE call_form='receiver'`, each edge re-evaluated through the shared
`ReceiverIndex`/`receiver_binds` gate (`rust/src/graph.rs`) — the one judge, never a copied
approximation of it. This yields a real per-call-site partition instead of a textual estimate.

**Placement — found by re-verifying the review against the sources.** The reviewer's fix says
"re-evaluate those rows through the shared gate" and names `recall-exp` as the home. The substance
is right; the placement bends a charter: `evals/src/recall.rs`'s module doc states the
counterfactual "does not open `cort`'s database and does not link `cort` at all ... never the
gate's output." The index-side table is therefore a **separate mode or sibling subcommand** that
composes the gate's own primitives — `ReceiverIndex::candidates` cardinality, `symbol_owner`,
`receiver_binds` (`rust/src/graph.rs`), the same shape `resolve_edge_targets` applies (`len() != 1`
refuses, then binds) — never a copy of their logic, and never a quiet repurposing of the
source-only tool. The evals crate already links `cort` (`cort::usage::machine_id()`), so the
dependency is not the obstacle; the charter is.

**Provenance — corrected after review.** Two first-draft claims were wrong.
`cort-evals recall-exp` stdout is *already* machine-stamped (`print_report` -> `stamp_machine`,
`evals/src/main.rs`); what is missing is the venue head. And `run_status_json` carries only batch
counts — it stamps no machine identity. So the work is: add the head to the report, have
`--report <path>` write the same stamped values into the markdown (render JSON and markdown from
one measured value, never a second computation), and require a current index — or print both the
venue HEAD and the stored index HEAD plus staleness, never labelling old indexed counts with the
current commit.

**Proposal.** A new `cort-evals gate-audit --venue DIR [--report out.md]` subcommand — the
sibling the Placement paragraph calls for, and the same command Item B's classes land in, since
population, gate re-evaluation and stamps are identical work; `recall-exp` itself is left
untouched with its counterfactual charter intact. With `--report`, alongside the stamped JSON on
stdout, write a markdown table. Columns: venue (path spelling as passed; a copy committed to
the repo must use a repo-relative path), venue head (`no-git` — the existing `venue_head` helper
*rejects* non-git venues, so this needs a tolerant variant), index head when it differs,
receiver edges in the population, attached, refused, and — once item B lands — refused-count per
class. The table is generated output committed by a deliberate act, like `evals/runs/`; nothing
auto-writes into the README. A number is quotable only with its row's commit, and the generator
enforces that by printing the commit on every row rather than once in a header. The textual
population `recall.rs` scans today stays available, labelled as the counterfactual it is.

**Acceptance.**
- `cort-evals gate-audit --venue DIR --report out.md` writes the table; without `--report`, only
  the stamped JSON on stdout; `recall-exp`'s own output is unchanged by this item.
- Every row carries machine id + source (via `stamp_machine`), the venue head, and the index
  head whenever the index is not current.
- The gate re-evaluation calls `rust/src/graph.rs`'s own gate — no second implementation of the
  binding decision anywhere in the table.
- Golden snapshot test for the rendering; option whitelists updated
  (`every_recognised_option_is_listed`, `every_option_the_parser_asks_for_is_whitelisted`).

**Files.** the same sibling module as Item B (`evals/src/gate_audit.rs`), `evals/src/main.rs`
(flag plumbing, tolerant `no-git` head); `raw_edges` is queried over a `SQLITE_OPEN_READ_ONLY`
connection, and the gate's decision stays in `cort::graph`'s primitives — no second SQL copy
of the binding decision. Per invocation, one venue; multiple venues means multiple runs, not a
merged table.

## Item B — classify the receiver-gate refusal population (measurement, not a feature)

**Problem.** The gate is name-based (`ReceiverIndex`, `rust/src/graph.rs`): it binds a receiver
call when the method name answers to exactly one local owner and the receiver looks like that
owner. What it cannot see by construction is *type-directed dispatch* — a receiver whose static
type is a trait, resolved through impls — and the refusals are where recall leaks. codegraph
proves such edges are shippable (their convention edges ship tagged `provenance:'heuristic'`),
but adopting that shape would trade our 117/117 precision story, so the decision needs a number
first — the same way the demand corpus decided callers-only.

**Classification — corrected after review.** The first draft's four classes were built on
wrong assumptions about the gate and were not disjoint: `ReceiverIndex` indexes every chunk by
the last segment of its symbol *including owned trait methods*, so "no local owner but matches a
trait-method declaration" is internally contradictory; and a unique owned method whose receiver
resembles the owner normally attaches, so calling that shape a routine "gate miss" was wrong.
The classes are the gate's own ordered refusal reasons, evaluated on indexed receiver edges:

1. zero candidates for the name;
2. multiple candidates;
3. one candidate, ownerless;
4. one candidate refused at the binding step — named `binding_refused` since the gate's order
   (shape, then ownership, then the name match) means a shape refusal can carry an ownerless
   candidate; `no_receiver_shape` counts those beside the class.

Trait/impl status is **dropped** — by the second review pass (2026-09-17), then confirmed against
the pack: both method rules emit the same `chunk:method`
(`cort-rust-chunk-impl-method`, `cort-rust-chunk-trait-default-method` in
`src/pack/rules/rust.yml`), and `cort-rust-chunk-type` emits struct, enum *and trait* declarations
alike as `chunk:class`, which the `chunks.chunk_type` CHECK enumerates flat. No bit in the index
distinguishes a trait from a struct, so the tag has no derivation — and a field that cannot say
how it was derived is decoration, not evidence. The type-directed *question* ("how much of class 4
is trait dispatch?") stays answerable the way this repo answers everything: by reading class 4's
examples, which `raw_edges` serves for free (`file_path`, `call_site_line`). If those examples
show trait shapes dominating, a proposal to carry trait-ness in the extractor gets written with
its own review — now with a measured reason.

One sub-count keeps class 4 honest: a receiver-form edge whose `raw_target` carries neither `.`
nor `:` is refused by `receiver_binds` on shape before any owner question exists, so it lands in
the binding-refused class by the gate's own logic but is **not** a type-directed-dispatch
candidate — and neither is a dotless *ownerless* candidate, which the gate also refuses on shape
first. Reason attribution therefore calls `receiver_binds` before attributing anything, reads
shape through the gate's shared `receiver_shape` primitive, and reports `no_receiver_shape`
beside the class, so neither malformed rows nor ownerless ones can inflate the class the whole
item exists to read.

**Explicitly out of scope.** Attaching any new edge kind, changing `rust/src/graph.rs`, changing
gate behavior, changing `impact`/`--coverage` output. Also out of scope: calling any class a
"true miss" — that requires independent ground truth, which syntax alone cannot supply. If class
4 measures large, that is a separate proposal with its own review.

**Acceptance.**
- The per-class breakdown partitions the indexed refused set; classes are disjoint and sum to
  refused; `no-receiver-shape` is reported beside class 4, never merged into it.
- Examples carry relative `file:line` taken from `raw_edges`' own columns — no source re-reading
  and no text-side plumbing is involved (the first draft's recall.rs plumbing bullet is obsolete:
  the indexed population carries attribution natively).
- Fixture test on a small synthetic venue covering all four classes, including an
  owned-candidate refusal and a `no-receiver-shape` row.
- No product *behavior* changes (amended by the implementation review): `graph.rs` gains exactly
  one thing — `receiver_shape`, the precondition `receiver_binds` already applied, extracted
  behavior-identically and made public so the census composes it instead of copying it, on the
  same grounds `hook-probe` replays the judge. `evals` already depends on `cort`
  (`evals/Cargo.toml`) and the gate primitives are public. There is no typed public raw-edge
  loader, so the eval side queries `raw_edges` with its own SQL over a
  `SQLITE_OPEN_READ_ONLY` connection (`db::open_db` creates directories, selects WAL and
  rewrites permissions — an audit that claims to never write cannot use it), while every
  *decision* (candidate cardinality, shape, ownership, binding) stays inside `cort::graph`'s
  primitives. Reading rows is not a decision copy.

**Files.** a new sibling module in the `evals` crate (proposed: `evals/src/gate_audit.rs` behind a
`cort-evals gate-audit --venue DIR` subcommand — the sibling placement honors `recall.rs`'s
source-only charter exactly as Item A's does), plus `evals/src/main.rs` plumbing and tests.
`recall.rs` is not touched.

## Item C — final context occupancy on run-agents rows — **DROPPED by its feasibility gate**

> **Gate outcome, 2026-09-16, machine `44a8a38f0b05b9a8` (etc-machine-id, via `stamp_machine`):
> DROPPED, per the pre-registered rule.** An earlier draft of this line stamped
> `046360e0bab8f003`, hand-derived as the sha256 of /etc/machine-id's first sixteen characters;
> the tool hashes the whole file, and the tool's stamp is the authoritative one — replaced here
> the day the discrepancy surfaced, so no figure in this doc carries a stamp that cannot be
> reconciled. Two real captures (same flags as `build_args`, `claude` 2.1.273; one
> nested in a Claude Code session, one with session env stripped), both on the machine's standing
> routing (`ANTHROPIC_BASE_URL` router, `ANTHROPIC_MODEL=glm-5.3-*`; `settings.json` says
> `model: opus` but env wins since the harness passes no `--model`): every assistant event
> carries a `usage` key *structurally*, but its value is `{"input_tokens":0,"output_tokens":0}` —
> while cumulative `result.usage` holds the real numbers (37,902 and 43,233 input tokens). The
> zeroing survives the session-env strip, so it is the backend's per-message reporting, not an
> artifact of how the capture was launched. Per the gate's own rule the item is **dropped, not
> approximated**: differencing the cumulative `result.usage` is exactly the forbidden second
> decision, and `final_context_tokens` cannot be filled on the machine that runs the harness.
> Scope of the claim: this is a property of the *backend's per-message usage reporting on this
> machine's routing*, not of the stream format — on a machine whose default backend reports
> per-message usage, the item can be re-proposed with a fresh capture and its own review. The
> capture files are transient (`/tmp`), which is fine: the finding is the shape, recorded here
> with the machine stamp; a fixture was never written because the gate failed before one was
> needed.

**Problem.** Rows record flow (`total_tokens`, `tool_return_tokens`) but nothing about what is
still sitting in the window when the answer is given. codegraph measured this axis and lost on
it (~80% more resident context than a file-reading agent); for `cort` it should be a win — lean
rows are small — and the number is one fold away.

**Problem.** Rows record flow (`total_tokens`, `tool_return_tokens`) but nothing about what is
still sitting in the window when the answer is given. codegraph measured this axis and lost on
it (~80% more resident context than a file-reading agent); for `cort` it should be a win — lean
rows are small — and the number is cheap.

**Definition — corrected after review.** The first draft called this "residual retrieval
context" and gave a two-term formula. Both were wrong: codegraph's residual number is derived
from per-turn deltas and attribution, which we are not building; what one retained field can
honestly carry is **final context occupancy** — the last usage event's
`input_tokens + cache_read_input_tokens + cache_creation_input_tokens`. Named
`final_context_tokens`. It states what the session ended holding, not what the tool cost.

**Mechanics — corrected after review, then hardened by re-verification.** The stream fold today
inspects usage only on the final `result` event (`evals/src/stream.rs`); assistant events
contribute tool calls only. That `result.usage` is a *cumulative* total — our own rows depend on
it, and codegraph's methodology doc says the same of theirs ("`result.usage` ... cannot answer the
occupancy question") — so the cumulative value cannot be differenced or reused here: the last
assistant turn's own `message.usage` is the only route to a per-turn snapshot. **Feasibility
gate:** no committed ndjson in `evals/runs/` shows that shape, so the claim "assistant events
carry `message.usage`" is unverified. The first implementation step is capturing one real stream
and checking it; if assistant events carry no usable usage there, this item is **dropped, not
approximated** — a final-context number differenced from the cumulative result usage would be a
second decision describing something that does not ship. The fixture must be that captured real
stream, not an invented one. Adding the field touches `REQUIRED_FIELDS` (fixed array),
`build_row`, and the numeric validation in `evals/src/arms.rs`; there is a count-golden
`METRICS.len() == 4` in `evals/tests/harness.rs` to update only if this becomes a strict metric.
Terms: "sidecars" was the wrong word — the metrics live in rows; `run-status.json` holds batch
counts only. Old rows simply lack the field.

**Proposal.** Record `final_context_tokens` per row. Summaries report
`mean_final_context_tokens` plus `final_context_tokens_measured` (the count the mean is over),
tolerate-absent on historical rows, outside the `--strict` gate until row versioning exists.

**Acceptance.**
- A captured real ndjson yields the intended value, not a sum over turns.
- Summaries tolerate rows missing the field; batch counts unchanged; `--strict` behavior against
  old rows is stated in the row doc.
- `cargo fmt --all`, clippy (warnings as errors), and `cargo test --locked --all-targets` green
  in both crates.

**Files.** `evals/src/stream.rs`, `evals/src/arms.rs`, `evals/src/summary.rs`,
`evals/tests/harness.rs`, tests.

## Item D — run-row machine identity (the one piece of C that survives the drop)

**Landed at `ced8795f` (2026-09-17), per-row.** Of the plan's two spellings — sidecar
(`run_status_json` + `BatchRead::report`) or row — the row won for one reason: `rows.json` is the
artefact that travels (`summarize` accepts bare rows.json paths and never sees the sidecar), so
provenance belongs on the thing that moves, where `REQUIRED_FIELDS` makes a row unwritable
without it. `build_row` writes `machine {id, source}` from `cort::usage`'s own accessors — the
same stamp `print_report` puts on reports — and the summary surfaces `row_machines`: one entry
per generating machine, id-sorted, rows predating the stamp disclosed as an absent bucket last.
The top-level stamp still names the *summarizing* machine; the two together make the 09-03
scenario visible in the aggregate instead of reconstructible after the fact. The row-side
statement of the choice lives where the plan asked, in the row doc (`evals/src/arms.rs`).

New run rows carry `venue_head` but no machine identity (`build_row`,
`evals/src/arms.rs`), and the summary's top-level stamp names the *summarizing* machine, not the
machine that generated historical rows — so a `rows.json` carried to a second machine and
summarized there is exactly the 2026-09-03 417-fires reconciliation problem again, one level up.
This item stamps runs: machine id + source into `run_status_json`, surfaced through
`BatchRead::report` (or onto every row — the implementer states which in the row doc, with the
same tolerate-absent handling for historical rows). Independent of C's drop: the gate killed the
*metric*, not the provenance.

**Ordering.** D is independent and small. B precedes A's per-class columns (A can ship without
them). A and B stamp machine + heads from the start.

## Rejected, with reasons (so review can police the boundary)

- **OS-watcher daemon auto-sync.** We are on WSL2; their own troubleshooting section is a catalog
  of the failure class this design avoids (WAL on `/mnt`, local-socket deaths, cross-boundary
  locks). The 2026-09-14 self-healing query (`rust/src/heal.rs`) strengthens the rejection: the
  query is the one who heals; a daemon would be a second repairer with its own liveness problem.
- **MCP surface, browser UI.** New surfaces make answers more numerous, not cheaper to verify.
- **`affected` (tests touched by changed files).** One filter over existing dependents; the demand
  corpus does not ask for it.
- **Callees direction.** The scope fact stands: no callee direction in the product; their flow
  questions are a different product's center.
- **Their per-language README table as marketing.** Adopted only as item A, with commit+machine
  stamped rows — the part their table lacks.
- **Their residual-occupancy metric as such.** Per-turn delta attribution is a methodology we are
  not importing; item C carries the one honest number a single field can hold.

## Constraints, restated as applied

Pure Rust in the `evals` crate; no scripts, no JS. No absolute paths anywhere, including fixtures
and committed reports. Storage failures returned, never panicked. `cargo fmt --all` + clippy
(warnings as errors) + `cargo test --locked --all-targets` in both crates before commit. Numbers
quoted only with their commit; machine id on every report; the gate's decision implemented once
(`rust/src/graph.rs`) and re-used, never approximated a second time.

## Open questions — answered by the review, kept for the record

1. *`--report <path>` vs a stdout format?* Keep `--report <path>`: it preserves the existing JSON
   stdout contract and makes file-write failures explicit. Render both from one measured value.
2. *Which number headlines?* Keep the gate-shaped totals (receiver sites, attached, refused);
   refusal classes sit beside them. No class is headlined as a "miss" — that needs independent
   ground truth.
3. *Backward compat?* Tolerate-absent on historical **rows** (not "sidecars" — corrected
   vocabulary): `mean_final_context_tokens` plus `final_context_tokens_measured`, outside the
   strict gate until row versioning exists.
