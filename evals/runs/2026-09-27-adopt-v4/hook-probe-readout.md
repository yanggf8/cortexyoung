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
| **First-search-only adjudication** | `PICK`, `V_BSR_GRP` (grep after `sed`/in pipeline) | never classified as a search at all |

Compound commands per se do NOT block: `grep … | head`, `grep …; echo ===; grep …` all fire.
The documented semantics "one Bash call = one verdict, the FIRST search" is what drops
`sed … | grep` and for-loop greps.

## Pricing the alternation lever (the proposed compound admission)

Of 5,547 distinct `pattern_not_symbol` patterns:

- 2,825 are alternations containing at least one bare identifier — but the top recoverable
  symbols are `FAILED` `readlink` `panicked` `TODO` `Duration` `SCHEMA_VERSION` — **log-scan
  vocabulary, not caller-set demand**.
- Tightening to camelCase/snake_case identifiers still leaves 1,739 candidates
  (`BOOK_TYPE` `BSR_STATUS` `Modify` `Running` …). The genuinely demand-backed symbols in
  there are about eight: `downloadCsv` `labelOf` `dialogFieldName` `gridHeaderName`
  `searchBSRList` `DateInput` `SettingSubuserController` `findHash`.
- Ratio ~200 noise : 1 demand, the evidence half cannot be calibrated offline, and every one
  of those projects is unindexed anyway (the hint, not `impact`, is what would ship).

## Verdict

1. **Do not ship alternation-splitting.** The precision-first stance (P4 copy, the `Taps`
   over-fire lesson) holds: the payoff is single-digit symbols behind three-digit noise and
   an unpriceable second gate.
2. **The bounded parser fix that is worth it: adjudicate past the first search segment**
   (`PICK`, `V_BSR_GRP` are the fixtures; `sed … | grep` never reaches the judge today).
   Small, testable, no precision blast radius — it only reaches searches the parser already
   understands, skipped today for position reasons.
3. **Index absence stays the first bottleneck** — of the 13 misses, 6 already pass shape and
   the remaining alternation symbols live in the same unindexed trees. Bootstrap
   agent-portal-web first; re-price the alternation lever only if misses persist with an
   index present.
