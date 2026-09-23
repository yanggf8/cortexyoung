# Hook demand and follow-through snapshot (2026-09-23)

These reports cover 2026-09-09 onward and use `adopt-mine-v3`. The full report keeps cortexyoung
visible for dogfood; `external-only.json` excludes its transcript directory. Read the two populations
separately.

## Readout

- Full set: 1,525 shell searches in 147 sessions produced 12 injections. One same-symbol `impact`
  was run within the follow window, but its triggering command explicitly says it is testing the
  hook, so it is not genuine dogfood adoption.
- The 12 linked instructions contain 0 `ask` and 0 `task` matches, 3 ordinary user instructions
  with no demand needle, and 9 prompts whose own words were stripped as pasted output or a bare
  directive. Those nine are **unknown**, not evidence of no need.
- Two injections had an editor action in the next five tool actions. One is the hook self-test above;
  the other is the external `Taps` event, a candidate for manual review rather than a confirmed
  benefit.
- External set: 1,332 searches, 9 injections, 0 adoptions; 1 ordinary instruction with no demand
  match and 8 unknown prompts. The 8 rows cluster in one project session, so they are not eight
  independent task examples.

## What this snapshot can answer

It identifies whether the hook reached a user task that the demand screen recognizes and whether an
edit followed soon after. `ask` / `task` are lexical labels; an editor action can be unrelated to the
hook. Review `injection_rows` before treating a candidate as positive. The reports are Claude Code
transcript data; the usage database also contains other harnesses and therefore cannot be used as a
matching denominator here.

## Re-run

```bash
cargo run --manifest-path evals/Cargo.toml --release -- adopt-mine \
  --since 2026-09-09T00:00:00Z \
  --out evals/runs/2026-09-23-adopt-v3/all-projects.json

cargo run --manifest-path evals/Cargo.toml --release -- adopt-mine \
  --since 2026-09-09T00:00:00Z \
  --exclude -home-yanggf-a-cortexyoung \
  --out evals/runs/2026-09-23-adopt-v3/external-only.json
```

The main result is a measurement gap: this window contains no confirmed caller-set demand. The next
useful sample is a real delete, refactor or review task in which the hook fires, the agent inspects
callers, and the ensuing edit is linked back to that task. More generic search injections would not
resolve the gap.
