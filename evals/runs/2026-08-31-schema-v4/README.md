# schema v4 — the counterfactual behind the receiver gate (2026-08-31)

Two claims were made in the commit message and in `docs/2026-08-31-recall-wip.md`. These files are
the evidence for them, so neither claim has to be taken on the author's word.

1. **"Unique name is not enough; it invents edges."** — `receiver-gate-counterfactual.json`.
   Computed from the shipped v4 index (`raw_edges`, `call_form = 'receiver'`) by re-running *only*
   the uniqueness half of the gate: 4,833 receiver call sites -> 30 with a project-wide unique
   method name -> 25 distinct (caller, target) edges, **13 real, 12 invented**. The shipped policy
   attaches 9 edges, 0 of them invented. Each row carries the source line it points at, so the
   grading is checkable against the file rather than against a model.
   The 12 invented ones are `kind`, `code`, `chain`, `matches`, `load` — the std methods a Rust
   project calls most, i.e. precisely the names most likely to be unique among a project's own
   symbols. That is the argument for `receiver_binds`, and any future relaxation has to answer it.
2. **"A reported call site is checkable against one line."** — `verify-impact-*.json`, produced by
   `cort-evals verify-impact` (the checker reads the file text, never the graph):
   * `verify-impact-cct-v3.json` — the five recorded cct chains, 117 dependents, indexed by the
     previous build: body `precision` 1.0 everywhere, and **no call sites**, because v3 stored none.
   * `verify-impact-cct-v4.json` — same venue, same symbols, re-indexed by the v4 binary:
     **identical dependent counts** (66/23/4/20/4) and `line_precision` 1.0 on 117/117. So the
     upgrade moved no TypeScript baseline: no TS rule changed.
   * `verify-impact-self-v4.json` — four Rust chains in this repo, 64 dependents,
     `line_precision` 1.0. The first entry also shows the *reason* the line check exists:
     `Tally::add` scores 0.667 on the whole-body check (the body says `tally.add`, the seed was
     asked for as `Tally::add`) and 1.0 on the line check.

## Reproducing

The v3 arm is a build of the parent commit, not an old cached database:

```
mkdir -p /tmp/cortbase && git archive <v4 commit>^ | tar -x -C /tmp/cortbase
cargo build --release --manifest-path /tmp/cortbase/rust/Cargo.toml
CORT_CACHE_DIR=/tmp/cort-exp-base /tmp/cortbase/rust/target/release/cort index   # in the venue
```

`cort-evals` takes the binary to check with `CORT_BIN` and the index with `CORT_CACHE_DIR`; the
venue is passed with `--repo`. Both caches are machine-local and are not part of this directory.

Caveats recorded honestly: the grading in (1) is a manual pass over 25 rows, by the same person who
wrote the gate. And neither check can see a *type* — a line that reads `e.kind()` "confirms" an edge
to any symbol named `kind`, which is why the gate refuses those on the receiver's shape instead of
trusting a unique name, and why `line_precision: 1.0` must never be quoted as correctness.

## These files are pinned to `a0269cda`, and what has moved since

`209fa06f` merged module-path suffix resolution and a Rust `edge:imports` rule on top of v4. Re-run
on that commit, same working tree: the cct arm is **unchanged** (1,839 relationships; 66/23/4/20/4
dependents with `line_precision` 1.0), and this repo's Rust chains gained 18 module-path edges
(+15 net) with six `AMBIGUOUS` rows narrowed by their own `use` path -- so
`verify-impact-self-v4.json`'s 64 dependents and the "1,369 relationships" quoted in
`docs/2026-08-31-recall-wip.md` §1 are figures for `a0269cda`, not for current master. The receiver
gate itself moved nothing: 9 attached edges before and after.
