# Claude Code cost census, 2026-09-17 to 2026-10-17 UTC

Run on machine `09a907a06cdea300` (`etc-machine-id`):

```sh
cargo run --locked --manifest-path evals/Cargo.toml -- cost-audit \
  --since 2026-09-17T00:00:00Z --until 2026-10-17T00:00:00Z \
  --out evals/runs/2026-10-18-claude-cost/report.json
```

The report reads the local Claude Code transcript tree, including subagent files, and records no
prompt, source text, command, or path. This is the first current-data screen for where Claude Code
spends tokens, **not** a savings estimate.

| Measure | Current window |
|---|---:|
| Session files with events in window | 1,085 (433 subagent files) |
| Unique assistant message ids | 65,643 |
| Unique messages with provider usage | 65,625 (18 without) |
| Unreadable files / unparsed lines / unmatched tool results | 0 / 1 / 3 |
| Estimated tool-result tokens | 39,033,805 |
| Bash tool-result tokens | 22,189,177 (56.8%) |
| Read tool-result tokens | 15,698,930 (40.2%) |
| Bash + Read share | 97.1% |
| Provider-reported cache-read tokens | 14,771,942,985 (98.0% of summed usage fields) |
| Top 1% of transcript files: model / tool-result token share | 65.5% / 19.4% |

The tool-result token estimate is ASCII characters divided by four plus one per non-ASCII
character, computed over result text. It is not a model tokenizer or a bill. Provider usage counts
every model turn's input, including context read repeatedly from cache, so its 14.77 billion
cache-read tokens must not be added to the 39.03 million tool-result estimate or interpreted as
avoidable cost. Assistant records repeat as content is appended; the reader deduplicates their
usage by message id and tool results by call id within each transcript file. Without that rule,
the usage sum is wrong before any interpretation begins.

The first word of Bash's command gives a useful, limited split: `sed` 5.17 million estimated
result tokens, `grep` 4.54 million, `cat` 1.46 million, and `git` 2.31 million; `other` is 5.00
million. The classifier skips an initial `cd … &&` and whitelists the first word, but compound
commands may run further programs, so these buckets are a screen rather than a semantic parse.
Together with `Read`, the `sed`/`grep`/`cat` buckets account for about 26.9 million estimated
tool-result tokens (69% of the total). That is an output surface, not yet a saving.

The stronger finding is concentration: the 11 most expensive transcript files by model usage
carry **65.5% of all summed model tokens**, while the 11 largest tool-output files carry only
19.4% of estimated tool-result tokens. These are *different* top-11 sets, and a transcript file
can span many user instructions; neither percentage identifies avoidable work. They show why a
tool-payload-only optimisation is a weak first bet. A sustained long-context or extra-turn loop
is the first mechanism to investigate. The reader counts each assistant message id once within
its file, so the concentration is not produced by repeated snapshots of the same message.

The next experiment should select real tasks from long files, label their successful outcome,
and compare the normal workflow against a bounded-context handoff on the same task and model.
Measure provider usage, turns, and success; include the handoff's own tokens. Separately test
compact `Read`/search output on the same tasks, so the two mechanisms are not confounded.
No task-success labels or no-assistance counterfactual are present in this census, so it cannot
say either intervention saves tokens yet.

