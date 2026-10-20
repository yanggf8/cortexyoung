# Direction after the first collection period, 2026-10-20

This is a priority decision from surviving measurements, not a claim that either product has
improved later answers. Cortexyoung owns reviewed project memory and the code-intelligence
screen. ClaudeCat remains a separate navigation and measurement tool that may call cort.

## What the collection says

- The 2026-10-19 telemetry upload scanned 13,851 structured events: 13,756 cort commands,
  90 daily cost aggregates, four historical ClaudeCat memory recalls, and one ClaudeCat
  navigation event (`claudecat/docs/2026-10-19-database-collection-audit.md`). These are
  observed tool events, not eligible questions or successful answers. Most cort rows predate
  per-row machine stamps; the upload reports that limitation rather than assigning them a
  precise machine retrospectively.
- A read-only `claudecat cort-audit --json` on 2026-10-20 reported 4,272 search-hook probes,
  5,223 edit-hook refreshes, seven `impact` calls, and 11 `memory` commands in the local
  30-day usage window. The seven-day window had no `impact` or `context` call. These usage
  totals come from one local usage database and are the same when the audit is invoked from
  either repository; they are not per-repository counts. This snapshot did not record which
  machine produced it. Re-run on machine `2eb02d46ec5c4319` the same day at
  `06bfe3f1`, the 30-day window held 4,031 search-hook probes, 4,424 edit-hook refreshes, 0
  `impact`, 0 `context` and 10 `memory` commands, so the two are not the same window or not
  the same machine; neither changes the priorities below. The memory commands include rollout
  and manual checks, so they cannot establish natural adoption. The search hook is silent by
  default, and its probe count cannot be read as delivered suggestions.
- `claudecat usage --json` reported one navigation event in its 30-day window on this
  machine. The earlier demand adjudication found only 4-7 of 1,214 genuine user instructions
  that required a call-site set (`docs/2026-08-31-demand-recheck.md`). The caller-set screen
  therefore remains a targeted aid for delete, refactor, and fact-check work, rather than a
  general answer surface.
- The Claude Code cost census found high model-token concentration in long transcript files
  and large Bash/Read tool-result volume (`evals/runs/2026-10-18-claude-cost/README.md`).
  Its archived totals have an unfixed historical timestamp-boundary error in the saved
  reports, and the census has no task-success counterfactual. It motivates paired workflow
  experiments, not a savings claim.
- On 2026-10-19, one physical machine recorded the reviewed `memory ownership` claim in
  `harnessuse`. On 2026-10-20, a second physical machine synchronized a cort-owned replica
  and recalled that active entry with `source_unchanged`, then read its cited line in
  `AGENTS.md`. This verifies one write-to-other-machine-read path. It does not test a
  correction written on one machine and read on the other, nor whether either harness
  answers a later task better. The ClaudeCat project's one active `memory database` entry
  currently reports `source_changed` after README edits; the current source still names
  `harnessuse`, but the entry needs reviewed supersession before use as fresh evidence.

## Priority 1: measure memory's effect on answers

Freeze a reviewed task set before changing recall behavior. Include past decisions,
superseded decisions, changed sources, and a false pasted agent report. Keep an answer key,
source snapshot, prompt, tool policy, and reviewer rubric for every task. Run fresh Claude
Code and Codex sessions with the same task and model settings in paired memory-on and
memory-off conditions. Record whether another native memory source was present. Report the
eligible-task denominator, observed recall and source reads, answer correctness, citation
correctness, justified uncertainty, regressions, turns, and tokens separately by harness and
machine. The existing ClaudeCat `memory eval-report` schema can check pair completeness and
artifact hashes; human review still judges the answers.

Treat an initial small set as a feasibility screen, not a recall or accuracy rate. A successful
sync, hook fire, or CLI recall is a plumbing result. Before building automatic capture or
unprompted retrieval, require paired tasks to show useful corrections in both harnesses
without hiding stale-source or false-report cases. Publish every regression with its source
check. Then repeat the result on an independent machine.

## Priority 2: make reviewed memory easier to maintain

Once the answer trial shows value, collect candidate decisions and corrections for human
review, with a committed source, scope, stable key, and explicit `--supersedes` link. Do not
promote transcript text or an agent's pasted report directly into the ledger. Count the
questions that should have consulted memory but did not, alongside actual recall calls. That
denominator, plus source-check and answer outcomes, decides whether an on-demand instruction
is sufficient or a narrower recall trigger is worth testing. Resolve the ClaudeCat entry's
changed source before including it in a fresh-source trial.

## Priority 3: test ClaudeCat's navigation on real work

ClaudeCat should remain the map, route, and evaluation front door; cort owns the memory table,
replica, and claim rules. Select real long tasks with labeled outcomes and compare the normal
workflow with a concise project entry point plus on-demand ClaudeCat navigation. Measure
success, turns, provider usage, and the handoff's own cost. Test compact file/search output in
a separate arm so it cannot be credited for a navigation result. One observed navigation call
does not yet support broadening map generation or injecting more context into every session.

Keep the caller-set graph maintained and check its edge and coverage disclosures on sampled
delete/refactor tasks. Expand receiver or trait-dispatch attachment only when those tasks show
specific missing callers whose proposed edges can be graded. Hook and graph volume alone are
not a reason to expand the feature.
