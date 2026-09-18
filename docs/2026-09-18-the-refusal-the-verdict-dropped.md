# the refusal the verdict dropped

date: 2026-09-18 · machine: this WSL2 box, linux x86_64 · tree: `f7a50464..02b1e30f` (pull), fix in this commit

The 0.45.2→0.45.3 pin bump (`6772e502`) reached this machine as a `git pull`, and
`cort_upgrade --check` answered about ast-grep with:

```
ast_grep: unreadable — no version output to read (inspect the named file — this never passes silently)
```

That message names no file and describes no state this machine was in: PATH held a perfectly
readable `ast-grep 0.45.2` at `~/.cargo/bin`. The same morning's
`docs/2026-09-18-cargo-fallback-shape.md` watched the author's machine read `drifted` for the
same bump — the difference between the two machines is one manifest key.

## §1 where the version went

`cort_upgrade` resolves the machine's ast-grep as: the manifest's `ast_grep_bin` (the
installer's OWNED-asset ledger) if present, else `resolve_ast_grep_bin()`. This machine
provisioned ast-grep before the installer ever ran, so there is no ledger key — and the
comment at the resolution site already explains why recording one would be wrong (uninstall
would delete a binary we never installed).

`resolve_ast_grep_bin()` fails closed on a pin mismatch: it returns
`Err(ast_grep_version_mismatch)` whose detail carries `found: "0.45.2"`, `expected:
"0.45.3"`, `candidate` — a version its own probe read off a binary that exists. The bin
flattened that Err with `.ok()`, and the empty string that survived judged Unreadable in
`check_version_pin`, whose Unreadable arm exists for output we could not read. We had read
it. **The Err is the machine's answer as surely as the Ok would be** — dropping it made the
verdict claim less than it knew, and the "inspect the named file" advice pointed at a file
nobody named. It is the same family as a figure that cannot say what produced it, one level
down: there the number cannot name its machine, here the verdict cannot name the version it
read.

## §2 the circle the advice drew

Drifted's `next_action` is "(rerun cort-upgrade, or --ack to accept)". ast_grep is the one
component that advice cannot fix: cort-upgrade judges it and never provisions it — repair is
install.sh's `install_ast_grep`, and a bare `./install.sh` declines (exit 3) on any machine
with a `cort_bin` manifest entry precisely to leave upgrades to cort-upgrade. This machine
ran that circle for real: one full `cort_upgrade` (which repaired binary and skills and left
ast_grep untouched), then a second look at an exit-1 verdict whose only failing row was
unreadable with nothing to inspect. The way out was `./install.sh --force` — the sanctioned
bypass, safe here only because the upgrade policy work had already landed in the same run;
the installer's release-asset download 504'd once and the retry verified the repo-maintained
SHA-256 before anything moved.

## §3 the fix

- `upgrade.rs` gains `ast_grep_mismatch_found(&CortError) -> Option<String>`: the version
  from the mismatch error's own `found`, and nothing else — `ast_grep_missing`,
  `"unparsable"`, and an absent field all return None, so Unreadable keeps meaning "claimed
  nothing". Pinned by `a_pin_mismatch_refusal_carries_the_version_the_probe_read`, including
  the composition assertion that a refusal-with-a-version judges Drifted with both sides
  named.
- the bin threads the Err arm into the version diagnose judges; the ledger key keeps
  precedence, and the Ok arm is unchanged.
- `next_action` points ast_grep drift at `./install.sh --force` instead of a rerun — the
  verdict now names the machine it wants and the tool that can change it.

Live reproduction on the fixed build, fixture HOME (no ledger key) + PATH holding only a
0.45.2 stub:

```
ast_grep: drifted — installed reports 0.45.2, tree pins 0.45.3 (run ./install.sh --force to provision the pinned ast-grep, or --ack to accept)
```

Same machine state, before and after; the second line is the one a reader can act on.
