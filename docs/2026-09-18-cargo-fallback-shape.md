# the cargo fallback finally has a shape to break

date: 2026-09-18 · machine: this Mac, arm64 · tree: 565b48ee

The 0.45.2→0.45.3 pin bump (`6772e502`, 2026-09-17) left this machine's ast-grep one pin
behind — a time gap, not a cross-machine one; `cort_upgrade --check` named it (`ast_grep:
drifted`) and the release-asset route fixed it. What followed was a misdiagnosis worth
recording, because its shape is reusable.

## §1 the retraction

A manual verification of the fallback ran
`cargo install ast-grep-cli --version 0.45.3 --locked`, cargo answered
`could not find ast-grep-cli in registry with version =0.45.3`, and the conclusion drawn was
"crates.io never published the CLI at this pin, so the cargo fallback is dead and only the
GitHub release asset works." Wrong on every clause:

- `ast-grep-cli` appears nowhere in this repo. The provenance table names the crate
  `ast-grep` (`rust/src/install.rs`), and that is what the fallback runs.
- crates.io carries `ast-grep` at 0.45.3 (published 2026-08-31, `rust_version` 1.88.0), so
  the "requires Rust 1.88+" prose in the die message is accurate too; the 1.98.1 in the
  bump commit's message is the verifying machine's toolchain, not the crate's MSRV.
- Real-route proof, 2026-09-18: `cargo install ast-grep --version 0.45.3 --locked --root
  <throwaway>` built and answered `ast-grep 0.45.3` (executables `ast-grep`, `sg`).

The error message named the crate that was typed, not the crate the script runs — cargo's
"could not find X" testifies about X's existence, and X was a typo borrowed from npm's
`@ast-grep/cli`. An error message cannot testify about a command nobody ran; it is the
same family as a figure that cannot say what produced it.

## §2 the gap that was actually there

The fallback branch (`install_ast_grep`'s else-arm) had zero coverage. The smoke sandbox's
fake curl/wget always fail — so *every* smoke download fails — but the fake ast-grep
already reports the pin, so `install_ast_grep` returns at "already present" and the branch
that would run `cargo install` is never entered. The argv — crate name, pinned version,
`--locked` — was asserted nowhere. A pin bump that broke any of the three would have
shipped green: Test 9 corrupts the download to prove SHA fail-closed, which dies before
the else-arm.

## §3 the fix

- `tests/install-smoke.sh` Test 9b (inserted after Test 9, per the `13b` no-renumber
  precedent): wrong version at the resolved path forces past "already present", the default
  curl failure drives the branch, and a locally-overridden fake cargo logs its argv and
  stands in for the installed crate by overwriting the resolved ast-grep with the pinned
  version — which is what a real `cargo install` does to the binary. Asserts: install
  exits 0; the log line is exactly `cargo install ast-grep --version 0.45.3 --locked`;
  the manifest records `ast_grep_bin`.
- `install.sh`'s die message interpolates `$prov_crate` instead of restating the name —
  bash keeps no second copy of a fact the provenance table owns, the same single-home rule
  `install_facts.rs` already guards for version, repo and checksums.

## §4 what CI can and cannot say here

The pin test prints `SKIP:` where the host has no ast-grep — by rule, tests do not reach
for host-specific binaries — so a clean CI runner is structurally silent about any
machine's installed tool. Per-machine drift is `cort_upgrade --check`'s verdict, and it is
the one that caught this machine's. CI's coverable half is the fallback's *shape*, and
that is exactly what Test 9b adds: offline, hermetic, and loud the moment a rename or a
pin bump breaks the argv.
