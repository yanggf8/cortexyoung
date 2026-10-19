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

**Boundary correction (2026-10-19):** these committed reports were generated before
`cost-audit` parsed timestamps as instants. Its earlier string comparison excluded events with
fractional seconds in the first second of `--since` and included such events in the first second
of `--until`. The code and regression tests are fixed; these historical JSON files have not been
regenerated, so the exact effect on their totals is unknown. Re-run the command above against
the original transcript population before treating the quoted totals as exact.

| Measure | Current window |
|---|---:|
| Session files with events in window | 1,085 (433 subagent files) |
| Unique assistant message ids | 65,643 |
| Unique messages with provider usage | 65,625 (18 without) |
| Unreadable files / unparsed lines / unmatched tool results | 0 / 1 / 2 |
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

## Independent-machine check (2026-10-18)

The same command and UTC window on machine `44a8a38f0b05b9a8` (`etc-machine-id`) produced
[second-machine-report.json](second-machine-report.json). This is a separate population, not an
extension of the first machine's census.

| Measure | First machine | Second machine |
|---|---:|---:|
| Transcript files in window | 1,085 | 60 |
| Unique assistant messages | 65,643 | 4,704 |
| Estimated tool-result tokens | 39,033,805 | 1,616,447 |
| Bash + Read share of tool-result tokens | 97.1% | 93.3% |
| Cache-read share of summed provider usage fields | 98.0% | 96.9% |
| Top 1% of files' share of model / tool-result tokens | 65.5% / 19.4% | 24.3% / 26.3% |
| Top 10% of files' share of model / tool-result tokens | 82.8% / 44.1% | 86.5% / 59.2% |

The second machine supports the observation that Bash/Read dominate tool-result volume and that
repeated context dominates summed provider usage. Its top-10% model concentration is also high,
but its top-1% share is much lower. With only 60 files, the top-1% group is one file, versus 11 on
the first machine; neither share identifies an avoidable task cost. The two proposed paired trials
still have zero pairs, so this check does not establish savings. The daily collector produced a
stamped aggregate for 2026-10-16 in a temporary directory (3 files, 80 assistant messages), but
the timer is not installed on this machine; ongoing collection has not been verified here.

