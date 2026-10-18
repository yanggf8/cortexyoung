# Claude Code token-saving trials

These are two separate experiments. The JSON files contain **no trial results yet**. Do not
combine their rows or infer savings from the observational cost census.

## Shared protocol

- Select at least 20 real, reproducible coding tasks per experiment, spanning several projects
  and including long tasks. Freeze each task's prompt, repo commit, model, success label, and
  grading rule **before** running either arm. Do not use the old caller-graph-only task set as
  the token-saving population.
- Run both arms for the same task at the same venue head and model. Randomize arm order; use an
  isolated checkout for any task that edits files. Save full stream transcripts and their SHA-256
  hashes, including provider usage and cost. A run with missing usage is not a zero-cost run.
- Grade task completion against the frozen label or test from the resulting artifact. Record
  `grade_evidence`; the agent's own statement that it succeeded is not a grade. Count every
  intervention setup call and handoff message in the treatment total.
- A pair is valid only when the treatment actually received its intervention. Put one pair into
  the relevant hypothesis file; `cort-evals cost-trial --file <file>` refuses null usage,
  mismatched models or heads, missing trace hashes, and mixed hypotheses. Fewer than 20 pairs
  always report `insufficient_pairs`. Even `eligible_for_review` still needs trace and label
  review before a causal savings claim.

Each arm object needs `model`, `venue_head`, `success`, `grade_evidence`, `trace_sha256`,
`cost_usd`, `turns`, and `usage` with numeric `input_tokens`, `cache_creation_input_tokens`,
`cache_read_input_tokens`, and `output_tokens`. The treatment also needs
`intervention_delivered: true`. Each pair needs a unique `task_id` and `task_version`.

## Handoff

The intervention is a **fresh session after a fixed checkpoint**, carrying only a bounded
handoff note and the stage-two instruction. The baseline continues the original session from
the same checkpoint. Stage one, including creating the note, is common to both arms and counted
in both totals. Stage two must have the same goal, repo state, and grading rule. This tests
whether reducing accumulated conversation beats the cost of summarising and re-establishing
context. It does not change tool-output policy.

## Compact output

The intervention bounds file/search output while keeping one continuous session. The baseline
uses the ordinary output path. The treatment trace must show that the output constraint was
actually followed; a prompt that was ignored is a failed intervention, not a negative result.
It does not change session boundaries. Compare entire-task provider usage, cost, turns, and
success, including any extra reads caused by compact output.

## Continued observation

`cort-evals cost-collect` persists each missing day among the previous **seven complete UTC days**
of aggregate Claude Code usage to `$XDG_DATA_HOME/cort-evals/claude-cost/YYYY-MM-DD.json` (or
`~/.local/share/cort-evals/claude-cost/`). It stores no prompts, commands, source text, or
transcript paths. Run it daily before Claude Code's transcript retention deletes the raw data.
The included user systemd timer runs at 01:30 UTC. Its `Persistent=true` setting catches a missed
timer firing, and the seven-day scan catches further missed dates. Use `--days 30` for a
backfill of up to 90 days, or `--day YYYY-MM-DD` for one older day; `--force` refreshes an
already collected day after a late transcript
sync. Install `cort-evals` and this user timer on each collecting machine, then run
`claudecat telemetry sync` after collection to copy the daily reports into the shared TursoDB.
Daily aggregates are descriptive monitoring, not control-arm results.
Backfilled days count only transcripts still present on this machine; a zero before local
retention is not proof that no sessions ran that day.
