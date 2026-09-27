# hook-probe replay readout (2026-09-27)

Method `hook-probe-v3`, all transcripts on disk, machine-stamped in `/tmp/hookprobe-full.json`
(not committed — this file carries the numbers). The replay judges the SHAPE half only
(`Evidence::Unknown` fires); the index half cannot be replayed, so every fire figure is an
upper bound.

## Baseline

- 135,976 commands -> 8,226 searches (6,833 shell / 1,393 structured) -> `shape_fired` 398
  (4.84% of searches), 239 distinct symbols, 101 confirmed seeds.
- Declines: `pattern_not_symbol` 6,829 (83%), `concrete_file_read` 454, `unindexed_extension`
  277, `non_source_target` 183, `target_not_source` 85.

## Where the 13 missed episodes died

| Cause | Symbols | Verdict |
|---|---|---|
| Shape PASSED, killed by no-index only | `c_form` `useHashRoute` `UserKey` `skip_permissions` `EmailUtil` `lifetour` | 6/13 — index, not judge |
| Alternation pattern declined wholesale | `branch_selector\|ng-table-counts`, `TLS\|FMS\|...`, `/Bsr/(searchBSRList\|...)` | parser takes whole pattern |
| Definition-prefixed pattern declined | `export function DateInput`, `export type BookingDetail` | **correct refusal** (P4: 5/11 ignores were definition lookups) |
| Concrete-file grep declined | `subuser` in App.tsx, `exportRange` in one .java | correct refusal |
| Cross-tree from `$HOME` session | `quota` (`--include=*.rs` into claude-code-router) | `non_source_target` |
| **First-segment-only parsing** | `PICK` (died at its `cd … &&` prefix) | never classified as a search at all |
| Alternation, even if reached | `V_BSR_GRP` (`"V_BSR_GRP\|V_BSR_DATA"`, `grep -nE` on one file) | would decline regardless of position |

Compound commands per se do NOT block: `grep … | head`, `grep …; echo ===; grep …` all fire.
Corrected wording (second pass): `search_from_shell` examines the FIRST PIPELINE SEGMENT
whatever it is (`rust/src/hook.rs:557`, `:624` — only `VAR=`/`sudo` prefixes are skipped), it
does not scan forward to the first search. So `sed … | grep …` AND `cd X && grep …` both
yield no search at all. The `cd && grep` form is common in real agent traffic: ~1,605
commands across all history never reached the parser (raw corpus count — an upper bound on
the population, not a would-fire count).

## Pricing the alternation lever (the proposed compound admission)

Of 5,547 distinct `pattern_not_symbol` patterns:

- 2,825 are alternations containing at least one bare identifier (command-unit; the
  second pass counts 2,823 commands / 2,743 distinct patterns) — but the top recoverable
  symbols are `FAILED` `readlink` `panicked` `TODO` `Duration` `SCHEMA_VERSION` — **log-scan
  vocabulary, not caller-set demand**.
- Tightening to genuine camelCase/snake_case (SCREAMING_CASE excluded) leaves 1,739
  commands / 1,702 distinct patterns (`BOOK_TYPE` `BSR_STATUS` `Modify` `Running` …). The
  genuinely demand-backed symbols in there are about eight: `downloadCsv` `labelOf`
  `dialogFieldName` `gridHeaderName` `searchBSRList` `DateInput` `SettingSubuserController`
  `findHash` — a manual classification from the transcript scan, not something the replay
  establishes.
- Ratio ~200 noise : 1 demand, the evidence half cannot be calibrated offline, and every one
  of those projects is unindexed anyway (the hint, not `impact`, is what would ship).

## Verdict

1. **Do not ship alternation-splitting.** The precision-first stance (P4 copy, the `Taps`
   over-fire lesson) holds: the payoff is single-digit symbols behind three-digit noise and
   an unpriceable second gate.
2. **The bounded parser fix that is worth it: consider the first segment that IS a search**
   (scan past leading non-search segments such as `cd X &&`). `PICK` is the fixture;
   `V_BSR_GRP` is NOT — its searches were alternations and would decline anyway. Precision
   is bounded: every newly reached segment still passes the existing judge (definition and
   concrete-file declines apply to it like any other). Note `commands_seen` drifts between
   runs (transcripts keep growing while we work) — treat it as unstable.
3. **Index absence stays the first bottleneck** — of the 13 misses, 6 already pass shape and
   the remaining alternation symbols live in the same unindexed trees. Bootstrap
   agent-portal-web first; re-price the alternation lever only if misses persist with an
   index present.

Second pass: Codex (session 01a0e31e-965c-7401-a2b8-156087eeb11b, 2026-09-27) — funnel
numbers reproduced exactly; the wording and fixture corrections above are theirs, verified
against `hook.rs` and the session transcript before being folded in.
