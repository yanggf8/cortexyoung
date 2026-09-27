# Hook demand and follow-through snapshot (2026-09-27)

Window 2026-09-23T00:00Z onward (post demand-capture, `8a632015`). Reports by `adopt-mine-v3`
plus two transcript scans (missed same-shape episodes; serveable-demand survey) and direct
`usage.db` queries. Numbers below are the corrected-window ones — an earlier hand-computed
epoch boundary was a year off and produced whole-database figures.

## Readout

- Funnel (claude-code): 4,786 fires -> 4,689 `no_shape` -> 26 symbol-stamped rows, ALL
  `no_index`/`no_index_hinted`. `hit`/`hit_stale`/`no_evidence`: **0**. Every symbol-shaped
  search of the window happened in a project without an index; the answer surface was never
  reachable from a hook fire. Direct `cort impact`/`context` usage in-window: also 0 (last
  `impact` 2026-09-14).
- Demand capture: 12 suggestion rows, 3 `captured` with own-words excerpts. Of the 3, one is
  code-shaped (`EmailUtil`, agent-portal-tpm); `skip_permissions` is a config key and `lifetour`
  a data concept — v:5's value is that the triage is now possible at all.
- Missed same-shape episodes (transcript scan, 59 sessions): 13 (11 strong identifiers), all in
  unindexed projects, vs the 3 hits. Distinguishing feature is command shape, not language or
  tool: all 3 hits were bare single-token `grep -rn "sym"`; the misses hid searches inside
  compound commands (`&&`/`;` chains, alternation, `-B/-A` context blocks, `--include`,
  `for` loops, sed pipelines). Corroborated on the usage side: the 09-26/27 compound greps
  left no symbol-stamped rows (died at `no_shape`), while `UserKey` with `-B6 -A30` did get
  stamped. Mixed evidence, small n — settle it with a `hook-probe` replay, never by hand.
- Class (a) (indexed project, judge declined) is the **empty set** this window: no indexed
  project had a work session. "Judge too strict on indexed projects" remains untested.
- Class (c): >=49 genuine demand messages where the agent never ran an identifier search at
  all (e.g. python-heredoc rewrite of 7 controllers) — no route in for the product.
- Compaction washed the demand original for 8 episodes (real symbols: `F_BLRLSDate`,
  `EDITABLE_FIELDS`, `insertBatchBsrRemark`, `MailSender`, ...) — capture cannot attribute
  through a summary boundary today.
- Serveable demand (2.5-week survey, 399 sessions, 1,615 user messages): fact-check of pasted
  reports containing call-graph claims ~95 (regex lower bound), diff/PR blast radius 41,
  everything else single digits. Users almost never ask relational questions outright (1).
  Near-miss: callee direction ("打到哪") recurs 3+ times in the agent-portal-tpm surgery-map
  spec, but it is PHP + legacy Angular — language coverage gates that demand before direction
  does.

## What to do with this

1. `hook-probe` replay over this window to price compound-command admission (the established
   calibration path; a hand-rolled matcher over-counted 48% once).
2. `verify-impact` that ingests a pasted report (extract "X calls Y"/"only Z uses" claims,
   grade each) — converts the largest serveable surface without new graph machinery.
3. Bootstrap an index where demand actually lives; agent-portal-web is the cheapest
   (.mjs/TS). agent-portal-tpm's cross-targets are PHP/Java — check pack coverage first.
4. Demand reader that walks back past compaction summaries.
5. Put real work in an indexed project so class (a) becomes measurable.

## Re-run

```bash
cargo run --manifest-path evals/Cargo.toml --release -- adopt-mine \
  --since 2026-09-23T00:00:00Z \
  --out evals/runs/2026-09-27-adopt-v4/all-projects.json

cargo run --manifest-path evals/Cargo.toml --release -- adopt-mine \
  --since 2026-09-23T00:00:00Z \
  --exclude -home-yanggf-a-cortexyoung \
  --out evals/runs/2026-09-27-adopt-v4/external-only.json
```
