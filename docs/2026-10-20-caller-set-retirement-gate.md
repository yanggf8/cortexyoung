# Retirement gate for the caller-set screen, 2026-10-20

This is a trial design, not a result. It decides whether `cort impact`, `cort context` and the
relationship graph behind them stay in the product or are retired. It is written before any run
so the decision rule cannot be fitted to the numbers afterward.

## Why a gate and not a vote

Usage is near zero. On machine `2eb02d46ec5c4319` (Thinkpade15) the 30-day `cort usage` window
on 2026-10-20 held 0 `impact` and 0 `context` calls against 4,031 `hook-suggest` probes and
4,424 `hook-refresh` runs. The 2026-10-20 direction note quotes 7 `impact` calls from an
unstamped local window; neither figure is a task denominator.

Low use does not settle value either way:

- Demand is rare by construction: 4-7 of 1,214 genuine user instructions needed a call-site set
  (`docs/2026-08-31-demand-recheck.md`), all on the delete / refactor / review path.
- Adoption is the stronger negative: sessions that carried the skill made 409 searches and zero
  `cort` calls. A tool that is right but never invoked delivers nothing.
- The positive evidence is synthetic. The 2026-08-30 graph runs
  (`evals/runs/2026-08-30-graph{,-sample2}/`) used hop-structured questions on one hub-dense
  venue with the cort arm told how to use the tool; `rg+Read` coverage fell below 1.0 in 7 of
  10 cells there. No run has used a real delete or rename with an oracle independent of cort.

The gate asks the one question that separates "keep" from "retire": **on real delete, rename
and move tasks, does having `cort impact --coverage` available make an agent miss fewer real
call sites or make fewer false "safe to remove" claims?**

## Scope

In scope for retirement: `impact`, `context`, the `relationships` extraction, and the eval
subcommands that exist only to grade them (`verify-impact`, `recall-exp`, `gate-audit`).

Out of scope: `cort memory` (the repository's long-term goal), the chunk index and its
`hook-refresh` repair (ClaudeCat's `navigate --cort` reads that index), and `cort read`.
`hook-suggest` measures demand for `impact`; its fate follows the gate's verdict.

`context` is not tested here. It has zero calls in the window and overlaps `cort read` and
ClaudeCat navigation. Its retirement is a separate proposal unless someone names a task it
answers that neither of those can.

## Task set

Mine tasks from git history, never from cort's own index, so the tool under test cannot choose
its own exam.

1. **Source.** Commits in at least three venues and two languages (this repo and ClaudeCat for
   Rust; cct for TypeScript) whose diff deletes a definition, renames it, moves it, or changes
   its signature. Record venue, commit, parent and symbol.
2. **Oracle.** At the parent commit, apply only the deletion or rename of the definition and run
   the compiler (`cargo check --all-targets`, `tsc --noEmit`). Every error location that names
   the symbol is a true call site. Cross-check against the call sites the real commit edited.
   A task whose two lists disagree is reviewed by hand and the resolution is recorded; a task
   with no compiler oracle (untyped JavaScript, macro-generated calls, string dispatch) is
   either dropped or kept in a separately labeled stratum that is never pooled.
3. **Strata.** Free functions; methods reached through a receiver, including trait dispatch
   (where the receiver gate is known to refuse, `docs/2026-09-17-gate-audit-census.md`);
   symbols whose true caller set is empty, to test "nothing uses it" claims.
4. **Size.** Freeze at least 30 tasks with every stratum represented. Run the first 10 as a
   feasibility screen; the screen checks plumbing and prompts and is not reported as a rate.
5. **Freeze.** Commit the tasks file with its SHA-256, each venue commit, the oracle output and
   any hand resolution before the first agent cell runs.

## Arms

Both arms run the same model, prompt, turn limit and venue snapshot, through `run-agents` with
PATH containment and `arm_held` on every row (`evals/src/arms.rs`).

- **A, normal workflow:** shell, `rg`, `ast-grep`, file reads. No `cort` on PATH and no cort
  skill in the config directory.
- **B, with the screen:** A plus `cort` against a fresh index of the frozen snapshot, and the
  shipped `skills/ast-grep/SKILL.md` guidance.

The prompt is the real request shape, for example "Delete `X` and fix every caller" or "Is `X`
safe to remove? List every call site", and asks for call sites as `file:line`. The arms are
not told they are compared. Run each task, arm and harness three times. Claude Code is the
primary harness; Codex repeats the full set if the Claude Code result falls in the middle band
below.

A third, agent-free measurement runs once per task: `cort impact --symbol X --depth 1
--coverage` alone, graded against the oracle. It separates "the screen is incomplete" from "the
agent did not use it".

## Measures

Report each separately by venue, stratum, harness and machine; never pool strata.

- **Missed call sites** (oracle sites absent from the answer) — the primary measure.
- **False call sites** (answer sites absent from the oracle).
- **False "safe to remove" verdicts** — a claim of no callers when the oracle has one. Any
  increase here in arm B is a regression regardless of the other numbers.
- **Uptake** — whether arm B called `impact` at all, and before or after its first search.
- **Standalone screen recall and precision** from the agent-free measurement, plus how many
  misses its `--coverage` rows disclosed.
- **Turns, tokens and cost**, including the index build in arm B.
- **`arm_held`** — a leaked cell is excluded and counted, never graded.

## Decision rule

The unit is the task: each arm's missed-site count for a task is the median of its three runs,
so one noisy run cannot decide a task. Totals are sums of those medians over the frozen set.
The thresholds below are chosen before any run and are not revised after one.

- **Retire** if any of these holds:
  - arm A misses fewer than 10 oracle sites in total, or fewer than 2% of them, so there is
    nothing for the screen to fix (a count floor as well as a rate, because 2% of a small
    set is one or two sites);
  - arm B's total missed sites are not at least 50% below arm A's;
  - arm B misses more sites than arm A on more than 10% of tasks, so a few hub tasks cannot
    carry a total that most tasks contradict;
  - arm B makes more false "safe to remove" verdicts than arm A.
- **Keep, frozen** if none of the retire conditions holds and uptake is at least half of B's
  cells. Keep means maintenance only: no new edge kinds unless a graded missed caller in this
  trial motivates it.
- **Middle band.** If the standalone screen beats arm A but arm B's uptake is below half, the
  tool works and is not reached. Run one time-boxed invocation trial (for example the
  `CORT_SUGGEST=1` delivery path on the same task set). If uptake does not reach half within
  that trial, retire.
- **Deadline.** If the frozen set and the first full run do not exist by 2026-11-30, retire by
  default. An unexercised screen is maintenance cost without evidence.

## What retirement changes

- Remove `impact`, `context`, the `relationships` extraction and the eval subcommands listed
  in scope; migrate the schema in `cort_upgrade` so existing indexes stay readable.
- Remove `hook-suggest` and its `HOOK_TARGETS` entries in the same change, so no harness keeps
  firing a probe for a verdict that no longer exists.
- Rewrite `skills/ast-grep/SKILL.md` to route caller-set work to `rg` and `ast-grep` only, and
  update README, this file's predecessors and ClaudeCat's skill where they name `impact`.
- Keep the chunk index, `hook-refresh`, `cort read` and `cort memory` unchanged.

## Threats

- **Contamination.** `AGENTS.md` and installed skills name cort. Arm A must run from a config
  directory with no cort skill, and every row records which skills were present.
- **Oracle gaps.** The compiler misses calls in dead `cfg` branches, macros and dynamic
  dispatch. Hand review covers disagreements; the unoracled stratum stays separate.
- **Memorization.** Venues the model may have seen are labeled; a result that holds only on
  them is not a keep.
- **Selection.** Commits are mined by diff shape, not by whether cort answered them well.
- **Machine and harness.** Every report carries machine id, harness, model and venue commit;
  numbers without them are not compared.

## Work needed before the first cell

- A miner that lists candidate commits by diff shape and emits the frozen tasks file.
- An oracle runner that applies the definition edit at the parent and collects compiler error
  sites.
- A `run-agents` task kind graded on `file:line` call sites instead of symbol names
  (`evals/src/grade.rs` grades `expected_symbols` today).
- A Codex driver for `run-agents`, needed only if the Codex repeat is triggered; the arms
  module builds a `claude` command line today (`evals/src/arms.rs`).

All of them are Rust subcommands in `evals/`, per the repository's no-scripts rule. Results go to
`evals/runs/<date>-retirement-gate/` with the tasks file hash, machine id and harness on every
row.
