# Fields and variants in the enumeration: the boundaries, written before anyone builds

2026-09-15. Status: design note, nothing here is built. Written because the improvement
loop's first pass over the v4 log (`hook-suggest` rows carrying `symbol`/`why`, deployed
2026-09-13 as 9820d9f5) closed with a classification that needs a ruling, and because the
ruling changes three subsystems at once — nobody should start it from a verbal summary.

## The evidence

Window 2026-09-13 → 09-15, all harnesses, `usage.db`:

- 23 hook rows carried a symbol. 12 `hit`, 10 `no_evidence/absent`, 1
  `no_evidence/leaf_in_index`.
- Of the 11 "the index cannot see it" rows, **7 trace to one root**: enum variants and
  struct fields are not in the extraction grain. `chunks` holds only `function` (839),
  `method` (149), `class` (123) — `fuel_left` (a field, 11 textual hits in GalaxyWarHero),
  `lane_quota_left` (8), `Interceptor` / `Marines` (`UnitKind` variants, 13/23),
  `Terrain::Planet` (the `leaf_in_index` row: `Terrain` is indexed, the variant is not).
- The 4 remaining rows are honest refusals (`frame_events`, `CCT_TEST_NOW`: zero textual
  hits in the searched project — the symbol was never that project's to index) and are not
  part of this note.
- Demand evidence, not just supply: the GalaxyWarHero survey that produced six of those
  fires opened with "*`fuel_left` 現況全貌:欄位定義、全庫所有讀取點與寫入點*" — a
  field-level caller set, asked by name, that `impact` cannot answer today.
- Same week, the other half of the grain: `Taps` fired on a seed that was
  `gwh-tools/src/resample.rs:90`'s resampling struct while the session asked about the
  `SoundId::Taps` sound variant. A name-only seed gate cannot tell. Shipped 3de5af3c: the
  suggestion now carries `defined at file:line`. That fix handles *wrong-symbol* fires; it
  does nothing for *absent* symbols, which is what the rest of this note is about.

## The proposal under discussion

Teach the Rust pack to extract two more definition shapes, so the 7 rows become answerable:

- fields as `Owner::field`;
- variants — as bare leaf, `Owner::Variant`, or both (boundary 2).

## Boundary 1 — what does `impact` even claim for a field?

The product sentence says *caller set*. A field has no callers. What it has is reads and
writes. If fields enter `chunks`, every consumer of the enumeration must decide which of
these it is promising:

- `impact --symbol fuel_left` promises "every site that touches this field, and
  `--coverage` names what the enumeration could not see". That is a read/write set, and the
  report's wording (currently "who calls it", the skill text, the README cost section) all
  say *calls* — each place needs a field-aware phrasing or the tool starts promising one
  thing and grading another.
- Alternatively fields keep a different verb somewhere — but a second command means a
  second thing for the hook to route to, and the hook's whole budget rests on there being
  exactly one verdict.

Stance: reads-and-writes *is* the caller set of a field; keep one command, and fix the
phrasing at the edges. But that is a ruling to make explicitly, not a drift into.

## Boundary 2 — variant occurrence shapes

A variant appears as construction (`UnitKind::Interceptor`), as a match arm
(`UnitKind::Interceptor => ...`), and as a bare path segment. The ast-grep shapes for these
are not the call shapes the current pack keys on, so `raw_edges.rel_type` needs either a
new relation (`constructs` / `matches`?) or an overload of `references`. The relation
choice propagates into: `coverage.rs`'s `extracted_but_unresolved` filter (which matches
`rel_type IN ('calls','references')` and would silently ignore the new relation until
updated — the same query `evidence_in` mirrors, so the two must move together), the
`--coverage` boolean's gap arithmetic, and `verify-impact`'s soundness grading.

Also: bare leaf or `Owner::Variant`? Bare leaves collide across enums (the `Taps` problem,
now visible in the suggestion instead of silent in the index); qualified names match how
agents actually search (`Terrain::Planet` reached the hook qualified). Stance: extract
variants qualified, and let the bare-name fallback live in `evidence_in`'s existing leaf
matching where it already is.

## Boundary 3 — the receiver gate and grading

The receiver gate attaches a receiver call to its owner when the receiver "looks like the
owner". Fields flow through variables of the owner's type (`unit.fuel_left`), which the
gate today has no opinion about. Either the gate learns field receivers (new machinery, new
calibration numbers for `hook-probe`) or field edges carry only unqualified/inline targets
and lean on `--coverage` for honesty. Deciding this late is how the receiver gate's 9-of-
4,833 refusal story gets a sequel.

`verify-impact` grades a printed edge against its call site. For fields the equivalent is
"the printed line reads or writes the field" — a different predicate, and until it exists,
field rows are unverifiable, which per the goal sentence means *not evidence*.

## Boundary 4 — volume, and the precision tension

GalaxyWarHero: 1,116 chunks today. Fields + variants across a Rust workspace roughly
double or triple definition rows, and every one becomes a symbol the hook can `hit` on.
Most of this week's variant/field fires came from a read-only survey session — the exact
fires the adoption funnel scores as *correct dismissals*. More grains means more fires of
that kind; the suggest copy must keep earning the dismissals cheaply (one glance), and
`adopt-mine`'s `writes_in_window` split is the instrument that says whether the trade is
paying.

## Boundary 5 — the cheaper alternative, deliberately on the table

Do not extract; answer anyway. `chunks_fts` already indexes chunk *content*, and a field
name appears inside the content of the methods that touch it. A `no_evidence` response that
adds "the name appears in N indexed chunks (file:line), ask there" would have answered 6 of
the 7 rows with a pointer instead of a caller set — without new grains, new relations, or a
new grading predicate. It is not the product sentence (no completeness claim, no edge
list), but it costs an afternoon, not a ruling across three subsystems. Worth doing first
if the alternative is waiting.

## What would settle it

- Pick boundary 1's wording. Everything else inherits it.
- If extraction: pick the relation vocabulary (boundary 2) and the grading predicate
  (boundary 3) *in the same change* — a grain whose edges cannot be graded is worse than
  no grain.
- Either way, run `cort-evals recall-exp` over the demand corpus before committing, so the
  0.08% / 0.33-0.58% relational-demand numbers from `docs/2026-08-31-demand-recheck.md`
  get a field-and-variant layer instead of a guess.
