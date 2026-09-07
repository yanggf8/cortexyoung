# Single-Home Install Facts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the shim template, the ast-grep provenance, and the manifest key-set exactly one home each, in Rust, so no future change can update one copy and ship the other.

**Architecture:** A new `rust/src/install.rs` module owns all three facts. Three installer-invoked verbs (`internal-shim`, `internal-ast-grep`, `internal-manifest-keys`) expose them, following the `hook-install` precedent ("installer-invoked … not a verb to type"). `install.sh` queries the just-built `$crate_bin` — never the installed `cort` — and holds no copy of any of the three. Where bash must name a key to write it, a Rust test parses `install.sh` source and asserts coverage against the Rust-owned set.

**Tech Stack:** Rust (`rust/` crate: new module, three dispatch arms, moved constant), Bash (`install.sh`, `tests/install-smoke.sh` — deletions only, no new decisions).

**Spec:** `docs/superpowers/specs/2026-09-06-cort-upgrade-design.md` §1 (the anti-drift rule and its three named violations)

**Plan 3b of 3.** Plans 1, 2 and 3a shipped. 3c (`cort-upgrade` itself) consumes this: it cannot diagnose components against desired state until desired state has one home.

## Global Constraints

- Repo is pure Rust; the only executable Bash is `install.sh` and `tests/install-smoke.sh`. This plan DELETES bash-held facts; it adds no logic to bash. The only bash additions are calls to the new verbs.
- `install.sh` runs under `set -euo pipefail`. Every changed line must be safe under `-e`.
- New verbs follow the `hook-install` convention exactly: entry in `KNOWN_COMMANDS` (`rust/src/main.rs:32`), entry in `usage_value` (`main.rs:48`) marked `(installer-invoked: …; not a verb to type)`, arm in `dispatch` (`main.rs:553`). The existing test `usage_documents_every_command_the_dispatcher_actually_knows` (`rust/tests/cli.rs:136`) enforces the three stay in step — read it before adding a verb.
- Run `bash -n install.sh && bash -n tests/install-smoke.sh`, then `bash tests/install-smoke.sh`, then in `rust/` AND `evals/`: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets`. Every command must exit 0 — report each exit code, do not pipe through tail. The smoke suite needs a fresh release binary first: `cargo build --release --locked --manifest-path rust/Cargo.toml`.
- No absolute developer paths anywhere, including fixtures.
- Every task's Step 5 deliberately breaks the implementation and confirms the new test goes red. That discipline has caught real gaps in every prior plan in this project.

## Scope

Three facts move. Deliberately NOT in this plan, with reasons:

- **Uninstall logic stays in bash.** Uninstall reads specific keys and applies bespoke removal per artifact; moving the removal dispatch to Rust is lifecycle ownership, which is 3c's job. What moves here is only the *knowledge* of which keys exist (Task 3), so 3c starts from an enumerated set.
- **`CORT_VERSION` keeps one guarded duplication.** The shim's `--version` line comes from `env!("CARGO_PKG_VERSION")` after Task 1, but `install.sh` still uses `$CORT_VERSION` for its banner and for `--check`'s comparison. Deriving it from the binary would change the header path; instead a Rust test asserts it equals `rust/Cargo.toml`'s version. A guarded duplication with a test is not a second home — it is one home plus an alarm.
- **No change to what gets installed.** Same payload, same skills, same hooks. Only where the facts live changes.

---

## File Structure

- `rust/src/install.rs` (NEW) — the one home: `AST_GREP_PINNED` (moved from `ast_grep.rs`), `AST_GREP_REPO`, `AST_GREP_ASSETS`, `render_shim`, `MANIFEST_KEYS`, `MANIFEST_LEGACY_KEYS`. Nothing here decides anything at install time; it only states what a release is.
- `rust/src/ast_grep.rs` — loses the constant; its three uses (`:128`, `:140`, `:211`) point at the new home.
- `rust/src/lib.rs` — `pub mod install;` between `indexer` and `pack` (the list is alphabetical).
- `rust/src/main.rs` — three dispatch arms, three `KNOWN_COMMANDS` entries, three usage lines.
- `install.sh` — deletions (pin, repo, checksum table, shim heredoc) plus three query calls against `$crate_bin`, and one reordered block.
- `rust/tests/install_facts.rs` (NEW) — the coverage tests that make the single home enforceable.
- `tests/install-smoke.sh` — unchanged, except it must keep passing; it is the integration proof that the reordered install still installs.

---

### Task 1: the shim template lives in Rust

**Files:**
- Create: `rust/src/install.rs` (shim part only; the module grows in Tasks 2-3)
- Modify: `rust/src/lib.rs`, `rust/src/main.rs`, `install.sh`
- Test: `rust/tests/install_facts.rs` (new file)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn render_shim(cort_home: &str) -> String` in `crate::install`; `cort internal-shim --cort-home DIR` printing it. Tasks 2-3 add siblings to the same module.

- [ ] **Step 1: Write the failing test**

Create `rust/tests/install_facts.rs`:

```rust
//! Install facts have one home: this crate. install.sh holds no copy of any of them, and these
//! tests are what makes that claim enforceable rather than aspirational.

use cort::install::render_shim;

/// The shim is three lines and every one of them is load-bearing: the `--version` intercept keeps
/// `--check` from executing the binary, and the absolute paths are resolved at exec time, which is
/// what makes the generation flip take effect for already-installed shims.
#[test]
fn the_shim_has_exactly_the_shape_the_installer_ships() {
    let shim = render_shim("/home/someone/.local/share/cortexyoung/cort");
    assert_eq!(
        shim,
        "#!/usr/bin/env bash\n\
         if [ \"$1\" = \"--version\" ]; then echo \"cort 0.1.0 (rust)\"; exit 0; fi\n\
         CORT_PACK_DIR=\"/home/someone/.local/share/cortexyoung/cort/pack\" exec \"/home/someone/.local/share/cortexyoung/cort/cort\" \"$@\"\n"
    );
}
```

**Do not hard-code `"0.1.0"` in the assertion.** The version line must be built from the same
`env!("CARGO_PKG_VERSION")` the implementation uses, or the test pins a version rather than a
shape. Write it as:

```rust
    let version = env!("CARGO_PKG_VERSION");
    let expected = format!(
        "#!/usr/bin/env bash\nif [ \"$1\" = \"--version\" ]; then echo \"cort {version} (rust)\"; exit 0; fi\nCORT_PACK_DIR=\"{dir}/pack\" exec \"{dir}/cort\" \"$@\"\n",
        dir = "/home/someone/.local/share/cortexyoung/cort"
    );
    assert_eq!(shim, expected);
```

Second test in the same file — the `CORT_VERSION` guard:

```rust
/// install.sh still spells a version for its banner and for --check's comparison. That string must
/// equal the crate version, or --check reports MISMATCH on a correct install. The shim itself no
/// longer depends on it; this test is the only thing keeping the two in step.
#[test]
fn install_sh_version_matches_the_crate_version() {
    let installer =
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    let line = installer
        .lines()
        .find(|l| l.starts_with("CORT_VERSION="))
        .expect("install.sh sets CORT_VERSION");
    let shell_version = line
        .trim_start_matches("CORT_VERSION=")
        .trim_matches('"');
    assert_eq!(shell_version, env!("CARGO_PKG_VERSION"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test install_facts`
Expected: FAIL to compile — `no module named 'install'` (or `unresolved import`).

- [ ] **Step 3: Write minimal implementation**

Create `rust/src/install.rs`:

```rust
//! Install facts: the single home for what a release is.
//!
//! Three things used to live in `install.sh` in bash, restated from sources Rust already owned:
//! the shim template, the ast-grep provenance, and the manifest key-set. Bash's copies were the
//! ones that shipped — `HOOK_TARGETS` already taught this repository what happens then
//! (`docs/2026-09-03-installer-dedup-and-attribution.md` §2, §3). Nothing in this module decides
//! anything at install time; it only states facts. `install.sh` queries them from the just-built
//! binary and writes files; that is all bash is for here.

/// Render the `$BIN_DIR/cort` shim for the given `CORT_HOME`.
///
/// Byte-identical to what the installer has always written: the `--version` intercept answers
/// without executing the binary (which is what `--check` parses), and the absolute paths resolve
/// at exec time rather than install time — that late binding is what makes the generation flip
/// (`install.sh`, Task 3a) take effect for already-installed shims.
pub fn render_shim(cort_home: &str) -> String {
    format!(
        "#!/usr/bin/env bash\n\
         if [ \"$1\" = \"--version\" ]; then echo \"cort {} (rust)\"; exit 0; fi\n\
         CORT_PACK_DIR=\"{cort_home}/pack\" exec \"{cort_home}/cort\" \"$@\"\n",
        env!("CARGO_PKG_VERSION"),
    )
}
```

Register `pub mod install;` in `rust/src/lib.rs` between `indexer` and `pack`.

In `rust/src/main.rs`, add the verb following the `hook-install` pattern exactly:

```rust
Some("internal-shim") => cmd_internal_shim(&args[1..], usage),
```

with:

```rust
#[derive(Parser, Debug)]
#[command(
    no_binary_name = true,
    disable_help_flag = true,
    disable_version_flag = true
)]
struct InternalShimArgs {
    #[arg(long = "cort-home")]
    cort_home: String,
}

fn cmd_internal_shim(args: &[String], _usage: &mut UsageEvent) -> Result<Emit, CortError> {
    let a = InternalShimArgs::try_parse_from(args.iter()).map_err(clap_fail)?;
    // Raw, not JSON: install.sh redirects this stdout straight into the executable file, and a
    // JSON string there would ship a quoted shim. This is the hook-install-all-lean precedent
    // (`render_emit`, `main.rs:256-271`) — a machine-read verb needs a raw-rendering branch, not a
    // payload the shell has to parse back out. Add `Some("internal-shim-lean")` to that match with
    // the same shape: `payload.get("lean").and_then(Value::as_str)`.
    Ok(Emit {
        render_command: Some("internal-shim-lean"),
        format: Format::Lean,
        payload: json!({ "lean": cort::install::render_shim(&a.cort_home) }),
    })
}
```

Check how `Emit` is constructed for a machine-consumed command (`hook-install --all --status
--lean` is the precedent — read `render_emit` first, as above, rather than inventing a shape).
The earlier draft of this step returned `Format::Json` with the shim inside a JSON string and told
install.sh to redirect that into the executable — review caught that this ships a quoted shim.
Add `"internal-shim"` to `KNOWN_COMMANDS` and a usage line marked
`(installer-invoked: renders the $BIN_DIR/cort shim for a CORT_HOME; not a verb to type)`.

In `install.sh`, replace the heredoc (`:858-868`):

```bash
  mkdir -p "$BIN_DIR"
  local shim="$BIN_DIR/cort"
  # The template lives in Rust (`rust/src/install.rs::render_shim`); this script holds no copy of
  # it. Queried from the just-built binary, never from the installed one — the installed cort may
  # be the generation being replaced.
  "$crate_bin" internal-shim --cort-home "$CORT_HOME" > "$shim.tmp" 2>/dev/null \
    || die "fresh cort binary cannot render its own shim"
```

Then extract the field the same way the script extracts every other machine-read value from this
binary. Read how `install.sh` currently parses `hook-install --all --status --lean` output and use
that exact mechanism — a second parsing convention is a second home for "how to read cort".

Finish the block unchanged (`chmod`, `mv`, `record_manifest`, smoke, info).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test install_facts`
Expected: PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Change one byte in `render_shim` (e.g. drop `"$@"`). Expected: the shape test RED.
2. Change `CORT_VERSION` in `install.sh` to a wrong value. Expected: the version test RED.
3. Delete `"internal-shim"` from `KNOWN_COMMANDS` while keeping the dispatch arm. Expected: the
   existing `usage_documents_every_command_the_dispatcher_actually_knows` test RED — proving the
   convention enforces itself.
4. Revert `install.sh` to the heredoc (keep the verb). Expected: **everything stays green** — and
   that is the gap. Neither new test observes whether the installer actually calls the verb, so a
   lazy implementation ships the renderer and keeps the old copy. Close it with a third test in
   `install_facts.rs`: assert `install.sh` source contains the `internal-shim --cort-home` call
   (via the same `include_str!`; match on the literal `internal-shim`). A test that only checks
   the renderer proves the home exists, not that anyone lives in it. Restore all four.

- [ ] **Step 6: Verify everything**

```bash
bash -n install.sh && bash -n tests/install-smoke.sh
cargo build --release --locked --manifest-path rust/Cargo.toml
bash tests/install-smoke.sh
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: all exit 0. The release build must precede the smoke run: with fake cargo the `internal-shim`
call in a full install is answered by the prebuilt tree binary, and these tests only mean something
if that binary contains this change.

- [ ] **Step 7: Commit**

```bash
git add rust/src/install.rs rust/src/lib.rs rust/src/main.rs rust/tests/install_facts.rs install.sh
git commit -m "feat(install): the shim template lives in Rust

install.sh held a three-line heredoc restating what the release is. Its copy
was the one that shipped, which is how HOOK_TARGETS rotted the binary's own
defaults into answering about the wrong file.

render_shim is now the one home, exposed as cort internal-shim following the
hook-install precedent for installer-invoked verbs. install.sh queries the
just-built binary -- never the installed one, which may be the generation
being replaced -- and holds no copy. A test pins the exact bytes, and a
second test pins install.sh's remaining CORT_VERSION string to the crate
version so --check cannot report MISMATCH on a correct install."
```

---

### Task 2: the ast-grep provenance lives in Rust

**Files:**
- Modify: `rust/src/install.rs`, `rust/src/ast_grep.rs`, `rust/src/main.rs`, `install.sh`
- Test: `rust/tests/install_facts.rs`

**Interfaces:**
- Consumes: Task 1's module and verb convention.
- Produces: `AST_GREP_REPO`, `AST_GREP_ASSETS`, `cort internal-ast-grep`; `install_ast_grep` reordered after the build.

**What "provenance" covers.** Everything `install_ast_grep` needs to decide *which* ast-grep to
fetch: the version pin (`AST_GREP_VERSION="0.45.2"`, `install.sh:12`), the repo
(`AST_GREP_REPO="ast-grep/ast-grep"`), the crate name for the cargo fallback (`AST_GREP_CRATE`),
and the per-asset checksum table (`sha256_for_ast_grep_asset`, `install.sh:52-60`). All four move.
What stays in bash is *how* to fetch: download, verify, unzip, install, fall back to cargo.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/install_facts.rs`:

```rust
/// Everything install_ast_grep needs to decide *which* ast-grep to fetch lives here. Today the
/// version, the repo, and the checksum table live in bash while `AST_GREP_PINNED` lives in
/// `ast_grep.rs` — two homes that are equal only by maintenance.
#[test]
fn ast_grep_provenance_names_the_pinned_release_and_its_checksums() {
    let prov = cort::install::ast_grep_provenance();
    assert_eq!(prov.version, "0.45.2");
    assert_eq!(prov.repo, "ast-grep/ast-grep");
    assert_eq!(prov.crate_name, "ast-grep");
    // Exact values, not mere presence: `checksum_for(asset).is_some()` accepts a changed checksum,
    // an empty checksum, even a constant `Some("")` — a break that keeps every assertion green
    // while shipping a lie. Each pair below is transcribed from install.sh's table; a single wrong
    // character must fail.
    for (asset, sha) in [
        ("app-x86_64-unknown-linux-gnu.zip", "67aff72dd2994bf152fcc3a8a09cf93b13193abe59f39393095167c729af2015"),
        ("app-aarch64-unknown-linux-gnu.zip", "TRANSCRIBE-FROM-FILE"),
        ("app-x86_64-apple-darwin.zip", "TRANSCRIBE-FROM-FILE"),
        ("app-aarch64-apple-darwin.zip", "TRANSCRIBE-FROM-FILE"),
    ] {
        assert_eq!(
            prov.checksum_for(asset),
            Some(sha),
            "wrong or missing checksum for {asset}"
        );
    }
}

/// install.sh must not name a version, a repo, or a checksum. It queries all three from the
/// just-built binary. Grep is the enforcement: these strings may appear in install.sh only inside
/// comments. The needle list covers every literal the old code held — the version, the repo, and
/// all four hashes — because checking one hash while three remain is a test that passes around the
/// defect. The crate name (`ast-grep`) is deliberately absent: it is also the binary name and
/// appears legitimately throughout the script, so banning the string is unimplementable; its home
/// is enforced by the provenance test above, not here.
#[test]
fn install_sh_names_no_ast_grep_version_repo_or_checksum() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    for line in installer.lines() {
        let code = line.split('#').next().unwrap_or("");
        for needle in [
            "0.45.2",
            "ast-grep/ast-grep",
            "67aff72dd2994bf152fcc3a8a09cf93b13193abe59f39393095167c729af2015",
            "TRANSCRIBE-HASH-2",
            "TRANSCRIBE-HASH-3",
            "TRANSCRIBE-HASH-4",
        ] {
            assert!(
                !code.contains(needle),
                "install.sh names {needle} in code: {line}"
            );
        }
    }
}
```

The checksum literal above is the x86_64-linux asset's SHA-256 from `install.sh:54`. If the table
ever changes, update this test from the Rust table — the test asserts bash holds no copy, so its
own copy of one hash is the exhibit, not a second home. (One hash suffices: the loop above already
asserts every asset has *a* checksum; this asserts bash has *none*.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test install_facts`
Expected: FAIL to compile — `no function named 'ast_grep_provenance'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/install.rs`:

```rust
/// Everything needed to decide *which* ast-grep to fetch: the version pin, the release repo, the
/// crate name for the cargo fallback, and the per-asset checksums for fail-closed verification.
/// The version pin used to live here twice over — once as `AST_GREP_VERSION` in bash and once as
/// `AST_GREP_PINNED` in `ast_grep.rs` — equal only by maintenance.
pub struct AstGrepProvenance {
    pub version: &'static str,
    pub repo: &'static str,
    pub crate_name: &'static str,
    pub assets: &'static [(&'static str, &'static str)],
}

/// The one home for ast-grep provenance. Copy the four asset names and hashes byte-for-byte from
/// the `sha256_for_ast_grep_asset` table install.sh carries today; the Task-2 test pins them.
pub fn ast_grep_provenance() -> AstGrepProvenance {
    AstGrepProvenance {
        version: AST_GREP_PINNED,
        repo: "ast-grep/ast-grep",
        crate_name: "ast-grep",
        assets: &[
            ("app-x86_64-unknown-linux-gnu.zip", "67aff72dd2994bf152fcc3a8a09cf93b13193abe59f39393095167c729af2015"),
            ("app-aarch64-unknown-linux-gnu.zip", "e67ee2f5928b4d77a472114ed4d7e8d933efcdf103a05e6706d7e8d933efcdf103a"),
            ...
        ],
    }
}

impl AstGrepProvenance {
    pub fn checksum_for(&self, asset: &str) -> Option<&'static str> {
        self.assets.iter().find(|(a, _)| *a == asset).map(|(_, s)| *s)
    }
}
```

**Copy the hashes from the file, not from the test above.** Two of the four literals sketched there
are placeholders by design — the test only needs one real exhibit, the implementation needs all
four real ones. Read `install.sh:52-60` and transcribe byte-for-byte; the new test fails on any
transcription error only if an asset name mismatches, so verify each hash by eye against the file.

Move `AST_GREP_PINNED` from `rust/src/ast_grep.rs:13` into this module and update its four
uses (`ast_grep.rs:128`, `:140`, `:211`, `:214`) to `crate::install::AST_GREP_PINNED` — not three;
review caught the error payload at `:214`. `rust/tests/ast_grep.rs:5` also imports the constant from
its old home; update that import and add both files to every `git add` and commit list in this task
that touches them. Delete the old constant — two names for one pin is the defect wearing a new coat.

Add the verb in `rust/src/main.rs` following Task 1 exactly (`KNOWN_COMMANDS`, usage line,
dispatch arm). Its payload carries version, repo, crate name and the asset table in the same shape
the hook-status precedent uses — check that precedent first, as in Task 1, rather than inventing
one.

In `install.sh`:

1. Delete `AST_GREP_VERSION`, `AST_GREP_REPO`, `AST_GREP_CRATE` (`:12-14`) and the whole
   `sha256_for_ast_grep_asset` function (`:52-60`).
2. Hoist the build, not the activation. The earlier draft moved the whole `install_cort` call after
   `install_ast_grep` — but that function also flips the symlink, writes the shim and smokes it, so a
   subsequent provisioning failure would leave the new generation activated with an old or missing
   parser. Instead extract the three build lines into `build_cort()` and call it before
   `install_ast_grep`, keeping the existing call inside `install_cort` (the second run is cargo's
   documented no-op — the comment at `:784-786` already says an up-to-date tree costs a fraction of
   a second). Everything `install_cort` needs still precedes it; only the compiler invocation moves.
3. Inside `install_ast_grep`, replace every use of the deleted variables with values queried once
   from the just-built binary at the top of the function:
   ```bash
   local prov
   prov="$("$crate_bin" internal-ast-grep ...)"  # same parse mechanism as Task 1's shim call
   ```
   Parse it with **the same mechanism Task 1 used** for the shim — one parsing convention for
   machine-read cort output, not two. The function needs `$crate_bin` in scope: it is currently a
   local of `install_cort` (`:782`). Promote it — define `crate_bin` once where both functions can
   see it, rather than recomputing the path twice.
4. Fix the `do_install` banner (`:1223`), which prints both versions before either is known. Print
   it after the build, from queried values, or drop the versions from it. Do not leave it reading
   deleted variables — under `set -u` that aborts the install.
5. Fix `do_check`, which this task would otherwise break: it expands `$AST_GREP_VERSION` at
   `:898`/`:901` but never runs `do_install`, so no queried value can supply it. Query the
   *installed* binary instead — `$managed_cort internal-ast-grep`, the same `unknown_command`
   fallback pattern `check_all_hooks` already uses at `:517`: if the installed cort predates the
   verb, print that the pin cannot be verified rather than comparing against a string that no
   longer exists. Deleting the variable without touching `do_check` aborts every `--check` under
   `set -u`.
6. The parse of the verb output must fail closed: the smoke suite's fake cort answers every unknown
   command with exit 0 and a generic string, so "exit 0" alone proves nothing. If the output does
   not parse as the expected shape, print the cannot-verify line — never compare the installed
   version against an empty pin, which would report MISMATCH on a correct install.

Be honest in the commit message about what the reorder buys: a doomed cargo build now fails before
the network download and before activation — not "before any mutation", because manifest migration
still precedes both. And a provisioning failure after activation leaves the new cort running against
an old or missing ast-grep; that is loud, not silent (`assert_ast_grep_version` fails closed at
runtime), but it is a real window and the message must not claim otherwise.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test install_facts`
Expected: PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Change one checksum character in the Rust table. Expected: RED on the provenance test —
   which now asserts exact values, so any single-character change fails. (An earlier draft asserted
   only `is_some()`, under which this break stayed green while shipping a lie; the honest limit that
   remains is a synchronized lie — Rust and test changed together — whose backstop is the smoke
   suite's Test 9 corrupt-download path plus `verify_sha` on a real download.)
2. Re-add `AST_GREP_VERSION="0.45.2"` to `install.sh` outside a comment. Expected: the no-copy
   test RED.
3. Swap the `do_install` order back (ast-grep before cort). Expected: the no-copy test still
   passes — ordering is invisible to it — so verify by reading instead: `grep -n
   'install_ast_grep$\|install_cort$' install.sh` must show the cort line first. If the order is
   wrong, provenance is queried from a binary that does not exist yet and the install dies. This
   one is enforced by inspection, and the plan says so honestly rather than claiming a test
   covers it.

- [ ] **Step 6: Verify everything**

Same commands as Task 1 Step 6 (release build first, then smoke, then both crates). All exit 0.
Watch the smoke test's ast-grep blocks: with fake cargo and no network, provisioning takes the
already-present path — read those blocks before concluding green means exercised.

- [ ] **Step 7: Commit**

```bash
git add rust/src/install.rs rust/src/ast_grep.rs rust/src/main.rs rust/tests/install_facts.rs rust/tests/ast_grep.rs install.sh
git commit -m "feat(install): ast-grep provenance lives in Rust

The version, the repo, the crate name and the checksum table lived in bash
while AST_GREP_PINNED lived in ast_grep.rs -- two homes equal only by
maintenance. All four now live in install.rs, exposed as cort
internal-ast-grep, and install.sh queries the just-built binary.

do_install builds cort before provisioning ast-grep, so the provenance is
always read from a binary that exists. The build step is hoisted, not the whole install: activation
still follows provisioning, and a provisioning failure after activation leaves the new cort running
against an old or missing parser — loud, not silent, because assert_ast_grep_version fails closed,
but stated here rather than hidden. A Rust test
asserts install.sh names no version, repo or checksum outside comments."
```

---

### Task 3: the manifest key-set lives in Rust

**Files:**
- Modify: `rust/src/install.rs`, `rust/src/main.rs`, `install.sh` (the `--check` advisory), `tests/install-smoke.sh` (one test for it)
- Test: `rust/tests/install_facts.rs`

**Interfaces:**
- Consumes: Tasks 1-2's module and verb convention.
- Produces: `MANIFEST_KEYS`, `MANIFEST_LEGACY_KEYS`, `cort internal-manifest-keys`. No install.sh
  behaviour change except through shared names.

**Why a test, not a query, enforces this one.** Bash must name a key to write it — the name has to
appear literally in `record_manifest "cort_bin"` — so the literals cannot leave the script. What
can leave is the *authority*: Rust owns the set of keys that may exist, and a test asserts every
literal install.sh writes is in it. A writer always knows what it writes; it is the readers (and
the future upgrader) that need one shared set. The test is the enforcement, and the plan says so
instead of pretending the strings are gone.

- [ ] **Step 1: Write the failing test**

Append to `rust/tests/install_facts.rs`:

```rust
/// install.sh must name a key to write it, so key literals cannot leave the script. What can leave
/// is the authority over which keys may exist: this set. The test parses every write site out of
/// install.sh and asserts membership here. A fresh install that grows a key this set does not know
/// fails here -- not in uninstall, not in upgrade, where it would surface as a leaked artifact.
///
/// There are two write shapes, and the test covers both, because covering one is how the other
/// hides: `record_manifest "literal"` writes directly, while `deploy_skill_at src dest "literal"`
/// flows its third argument into the generic `record_manifest "$key"` call
/// (`install.sh:456-457`, `:497`). A test that parses only the first shape observes nothing about
/// the skill keys and passes while they drift.
#[test]
fn every_manifest_key_install_sh_writes_is_known() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    let mut unknown = Vec::new();
    for line in installer.lines() {
        let code = line.split('#').next().unwrap_or("");
        // Shape 1: record_manifest "literal" — skip "$..." (dynamic; covered below via its source).
        let mut rest = code;
        while let Some(start) = rest.find("record_manifest \"") {
            rest = &rest[start + "record_manifest \"".len()..];
            if let Some(end) = rest.find('"') {
                let key = &rest[..end];
                if !key.starts_with('$')
                    && !cort::install::MANIFEST_KEYS.contains(&key)
                {
                    unknown.push(key.to_string());
                }
                rest = &rest[end + 1..];
            } else {
                break;
            }
        }
        // Shape 2: deploy_skill_at src dest "literal" — the literal becomes "$key" downstream.
        if let Some(start) = code.find("deploy_skill_at ") {
            let args: Vec<&str> = code[start..].split('"').collect();
            // args[1], args[3], args[5] are the three quoted arguments; the key is the third.
            if args.len() >= 6 && !args[5].starts_with('$') {
                if !cort::install::MANIFEST_KEYS.contains(&args[5]) {
                    unknown.push(args[5].to_string());
                }
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "install.sh writes manifest keys Rust does not know: {unknown:?}"
    );
}

/// The same for every key it reads. Reads come in three shapes: `manifest_get name` as a bare
/// word, `manifest_get "name"` quoted, and `manifest_get "$key"` where the key is a loop variable.
/// The bare and quoted forms are checked directly; the variable form is covered by checking the
/// loop that feeds it — `for key in hook_settings hook_settings_codex hook_settings_kimi`
/// (`install.sh:644`) — whose every word must be known. A whitespace-split test observes none of
/// this: every real call nests inside `$(...)`, so the token after a split is `cort_bin="$(manifest_get`,
/// never the key. Legacy keys (seen only via `manifest_get`, never written by a fresh install)
/// belong to MANIFEST_LEGACY_KEYS, not the main set.
#[test]
fn every_manifest_key_install_sh_reads_is_known_or_legacy() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    let mut unknown = Vec::new();
    for line in installer.lines() {
        let code = line.split('#').next().unwrap_or("");
        // Shape 1: manifest_get name or manifest_get "name" — never manifest_get "$var".
        let mut rest = code;
        while let Some(start) = rest.find("manifest_get") {
            rest = &rest[start + "manifest_get".len()..];
            let arg = rest.trim_start_matches([' ', '\t']);
            let key = if let Some(q) = arg.strip_prefix('"') {
                q.split('"').next().unwrap_or("")
            } else {
                arg.split([' ', '\t', ')', ';'])
                    .next()
                    .unwrap_or("")
            };
            if !key.is_empty() && !key.starts_with('$') {
                if !cort::install::MANIFEST_KEYS.contains(&key)
                    && !cort::install::MANIFEST_LEGACY_KEYS.contains(&key)
                {
                    unknown.push(key.to_string());
                }
            }
            rest = arg;
            if rest.len() < 2 {
                break;
            }
            rest = &rest[1..];
        }
        // Shape 2: the loop feeding manifest_get "$key" — every word after `in` is a key.
        if let Some(in_pos) = code.find("for key in ") {
            for word in code[in_pos + "for key in ".len()..]
                .split([' ', '\t', ';'])
                .map(str::trim)
                .filter(|w| !w.is_empty() && !w.starts_with('$') && *w != "do" && *w != "{" && *w != "")
            {
                let key = word.trim_matches(';');
                if !cort::install::MANIFEST_KEYS.contains(&key)
                    && !cort::install::MANIFEST_LEGACY_KEYS.contains(&key)
                {
                    unknown.push(key.to_string());
                }
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "install.sh reads manifest keys Rust knows nowhere: {unknown:?}"
    );
}
```

Delete the empty `for token` loop above before running — it is a leftover sketch of an approach
that does not work (split_whitespace loses the quoting structure the `record_manifest` parser
above relies on). It is left here visibly so the implementer does not re-derive it: **do not
"fix" it, delete it.** If that instruction is unclear, stop and report rather than improvising.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd rust && cargo test --test install_facts`
Expected: FAIL to compile — `no constant named 'MANIFEST_KEYS'`.

- [ ] **Step 3: Write minimal implementation**

In `rust/src/install.rs`:

```rust
/// Every key a fresh install may write to the manifest. This is the authoritative set: install.sh
/// must name a key to write it, so the literals stay in the script, but nothing may exist here
/// that is not named below, and the test above enforces it. A key added to the script without
/// being added here fails the build — not uninstall, not upgrade, where it would surface as a
/// leaked artifact.
pub const MANIFEST_KEYS: &[&str] = &[
    "manifest_version",
    "cort_bin",
    "ast_grep_bin",
    "legacy_xg_bin",
    "skill_xgrep",
    "skill_ast_grep",
    "skill_ast_grep_codex",
    "hook_settings",
    "hook_settings_codex",
    "hook_settings_kimi",
];

/// Keys no fresh install writes but old manifests may hold, renamed by `migrate_manifest_v2`.
/// Readable, never written. Uninstall must still honour them, which is why they are named rather
/// than forgotten.
pub const MANIFEST_LEGACY_KEYS: &[&str] = &["xg_bin", "skill"];
```

**Derive this list; do not trust it.** Run the writers/readers enumeration yourself
(`grep -oE 'record_manifest "[a-z_]+"' install.sh | sort -u` plus the `deploy_skill_at` third
arguments, and the `manifest_get` form) and diff it against the list above. Two facts the earlier
draft of this paragraph got wrong, corrected here: `skill_ast_grep` and `skill_ast_grep_codex` are
**fresh-install** keys (written through `deploy_skill_at` at `install.sh:1257-1258`), **not**
migration arrivals — migration renames only `xg_bin` and `skill` (`install.sh:380-392`), so the
legacy set is exactly those two. If your enumeration finds a key in either direction that is not
in one of the two lists, the list is wrong and the test is what catches it — report the key
rather than silently extending the list, so the addition gets a second pair of eyes.

Add the verb in `rust/src/main.rs` following Tasks 1-2 exactly (`KNOWN_COMMANDS`, usage line,
dispatch arm). Its payload is the two lists in the hook-status shape — same parsing convention as
the other two verbs, not a third. And it gets a real caller today, not just a future one: `do_check`
gains an advisory line reporting manifest keys the binary knows nowhere (parsed from the verb
output; unknown keys never fail the check — they are information, and failing on them would strand
machines whose manifests predate this release). Fall back with the `unknown_command` message when
the installed binary predates the verb, following the pattern `check_all_hooks` already uses. Add a
smoke assertion for that advisory line using a manifest that carries one unknown key — back it up
first and restore it afterwards, the way the manifest tests isolate their fixtures.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd rust && cargo test --test install_facts`
Expected: PASS.

- [ ] **Step 5: Verify each test can actually fail**

1. Add `record_manifest "smoke_probe" "value"` anywhere in `install.sh` outside a comment.
   Expected: the writes test RED naming `smoke_probe` — through the literal arm, which is also
   what proves the `deploy_skill_at` arm is not the only one working.
2. Remove `"hook_settings_kimi"` from `MANIFEST_KEYS`. Expected: **both** tests RED. Every
   `hook_settings_*` key is written (the `case` at `install.sh:603-605`) and read, so a removal
   must fail on both sides; if only one goes red, the other test is not observing what it claims.
3. Delete the empty `for token` loop as instructed. Expected: still green — it was dead code in
   a test, and its removal must not change the verdict. If anything goes red, the loop was load-
   bearing and the plan misunderstood it; report that instead of restoring it.
4. Remove the `--check` advisory call (keep the verb). Expected: the new smoke test RED — proving
   the advisory is observed, not merely printed somewhere nothing reads.

- [ ] **Step 6: Verify everything**

Same commands as Task 1 Step 6. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add rust/src/install.rs rust/src/main.rs rust/tests/install_facts.rs install.sh tests/install-smoke.sh
git commit -m "feat(install): the manifest key-set lives in Rust

install.sh must name a key to write it, so the literals stay in the script.
What moved is the authority over which keys may exist: MANIFEST_KEYS for what
a fresh install writes, MANIFEST_LEGACY_KEYS for what old manifests may hold.
Two Rust tests parse install.sh source and assert every written key is in the
first set and every read key is in one of the two. A fresh install that grows
a key Rust does not know fails the build -- not uninstall, not upgrade, where
it would surface as a leaked artifact. The verb gets a caller today rather
than waiting for 3c: --check reports manifest keys no release knows, advisory
only, so the key-set is consumed authority rather than a list with tests."
```

---

## Self-Review

**Spec coverage.** Implements the three named violations in spec §1's anti-drift rule: the shim
template, the ast-grep pin (plus repo, crate and checksums, which are the same fact), and the
manifest key-set. §9's item 3 — `--status`'s `wired` being weaker than the output implies — is
explicitly deferred to 3c's component diagnosis, as plan 3a's Self-Review already states.

**The honest asymmetry, stated once.** Tasks 1 and 2 remove every copy from bash. Task 3 does not:
bash keeps key-name literals, and a test enforces coverage. That is not a weaker outcome wearing
stronger language — a writer must name what it writes, and the failure this prevents (an unknown
key leaking through uninstall or upgrade) is caught at exactly the point where the knowledge lives.
The plan says this in Task 3's own Interfaces-adjacent paragraph so a reviewer meets it before the
code, not after.

**Known gap, recorded rather than hidden.** A synchronized lie — the Rust table and the test
needles changed together — stays green by construction; the tests detect drift *between* homes,
not a falsehood both agree on. Two backstops, both real: the smoke suite's Test 9 corrupt-download
path (which exercises `verify_sha` failing, though with corrupt bytes rather than a wrong table),
and `verify_sha` on the first real download from the new tree, which is fail-closed.

**Placeholders:** the Task 2 checksum table carries two deliberately marked placeholder hashes with
an instruction to transcribe from the file — that is a transcription task with its verification
named (the asset-name test fails on a mismatched name; eye-check the hashes), not a TBD. Everything
else is complete code. The Task 3 test's dead `for token` loop is intentional: it documents a wrong
approach in place so nobody re-derives it.

**Type consistency:** `render_shim(cort_home: &str) -> String`; `ast_grep_provenance() ->
AstGrepProvenance` with `checksum_for(&self, asset: &str) -> Option<&'static str>`;
`MANIFEST_KEYS: &[&str]`, `MANIFEST_LEGACY_KEYS: &[&str]`; three verbs named
`internal-shim`, `internal-ast-grep`, `internal-manifest-keys`.

---

## What the review changed (Codex, 2026-09-07)

Nine findings; all nine verified against source before accepting, seven fully, one partially, one
refuted as stated but true underneath. The plan above already incorporates every accepted one — what
follows is the record, so a future reader can tell which sentences exist because review put them
there.

1. **The writer test captured `$key`.** `deploy_skill_at` takes the key as `$3`
   (`install.sh:456-457`) and writes it via the generic `record_manifest "$key"` (`:497`), so the
   test as written failed on unchanged legitimate code — and worse, the real skill keys at
   `:1257-1258` flow through that same variable. Fixed by covering both shapes: literals, plus
   `deploy_skill_at` third arguments.
2. **The reader test observed zero reads.** Every real `manifest_get` nests inside `$(...)`, so a
   whitespace split never yields the key as a token. Fixed with a targeted parse (bare/quoted
   argument after the command name, `$`-prefixed skipped) plus the `:644` loop words.
3. **`do_check` would have broken under `set -u`.** It expands `$AST_GREP_VERSION` (`:898`/`:901`)
   but never runs `do_install`. Fixed by querying the installed binary with the same
   `unknown_command` fallback `check_all_hooks` already uses.
4. **The constant move missed a use and a file.** `ast_grep.rs:214` is a fourth production use, and
   `rust/tests/ast_grep.rs:5` imports the constant — neither was in the task's file lists. Both are
   now named, including in the commit.
5. **The shim verb had no render path.** `Format::Json` would have shipped a quoted shim; the
   machine-read precedent (`hook-install-all-lean`, `main.rs:259-267`) is raw TSV through a
   `render_emit` arm. The verb now follows it, and a fourth break (revert to the heredoc, watch
   nothing go red) became a consumption test asserting `install.sh` invokes the verb.
6. **The checksum break could not go red.** `is_some()` accepts any value, so a one-character change
   stays green — a Step-5 prediction the plan made and its own Self-Review contradicted in the same
   file. Fixed by asserting exact values for all four assets; the remaining synchronized-lie limit
   is stated plainly with its two real backstops.
7. **Partially accepted: guarded duplication.** The reviewer is right that the verb had no caller
   and the membership tests prove less than the architecture claimed. Fixed by giving the verb a
   real one: `--check` reports unknown manifest keys, advisory only. What is *not* accepted is the
   demand that authority mean consumption everywhere — a writer must name what it writes, and the
   enforced-set-plus-advisory design is stated as such rather than reworded to sound stronger.
8. **Accepted and added: the lazy Task 1.** A renderer plus an unused verb passes every test the
   plan had. There is now a consumption test (break 4 above).
9. **Three false statements corrected.** The skill keys arrive via fresh install
   (`deploy_skill_at`, `:1257-1258`), not via migration — only `xg_bin`/`skill` do, so the legacy
   set is exactly those two. Moving `install_cort` would move activation, not just the build — so
   the plan hoists only the three build lines, and says honestly that a provisioning failure after
   activation is loud (`assert_ast_grep_version`) rather than claiming nothing can go wrong. And the
   "backstop untested" claim was wrong: the smoke suite's Test 9 corrupt-download path does exercise
   `verify_sha` failing.
