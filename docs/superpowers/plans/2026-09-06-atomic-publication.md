# Atomic Publication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the installer destroying the old state before it has built the new one, in the four places it does that today.

**Architecture:** Every publication becomes stage → validate → activate → cleanup. The manifest and each skill are written to a temporary file and renamed over the live one; the payload becomes a versioned generation directory that a symlink points at, flipped by one rename; and the whole mutating sequence takes a lock so two installers cannot interleave.

**Tech Stack:** Bash (`install.sh` and `tests/install-smoke.sh` — the two files the platform requires; no logic moves in, and the diff removes more branching than it adds).

**Spec:** `docs/superpowers/specs/2026-09-06-cort-upgrade-design.md` §3(a), §5, §9

**Plan 3a of 3.** Plans 1 and 2 shipped. 3b moves the shim template, ast-grep pin and manifest key-set to a single home in Rust; 3c is `cort-upgrade` itself. This one comes first because the other two publish through this machinery, and because it is worth having on its own: after it, every existing install and re-install is crash-safe.

## Global Constraints

- `install.sh` and `tests/install-smoke.sh` are the only executable Bash, and **no logic may grow in them**. This plan adds file-placement mechanics, which is what bash is here for; it moves no decision into bash and removes one branch for every one it adds.
- `install.sh` runs under `set -euo pipefail`. Every construct below must be safe under `-e`: a bare `[ x = y ] && cmd` whose test is false returns 1 and aborts the script. Use `if`.
- Run `bash -n install.sh && bash -n tests/install-smoke.sh` before every run, and `bash tests/install-smoke.sh` must exit 0.
- Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and `cargo test --locked --all-targets` in **both** `rust/` and `evals/` before every commit even when no Rust changed — the smoke test builds the binary, and a green Rust suite is what makes its failures attributable. Never end a verification pipeline in `tail`.
- No absolute developer paths anywhere, including fixtures.

## Why these four, and why together

Verifying the five findings carried in spec §9 turned up one shape rather than five defects:

| Where | Today |
|---|---|
| `record_manifest` (`install.sh:313-322`) | `cat "$tmp" > "$MANIFEST_FILE"` **truncates the live file**, deletes the temp copy, then appends the key. Interrupted between those, the key is gone — from the only record uninstall has of what exists. Called ~9 times per run. |
| `write_skill` (`install.sh:216-219`) | `cat "$1" > "$2"` truncates the live skill, then stamps it separately. Interrupted, the file is partial and its old stamp no longer matches, so the next preflight calls it an **unmanaged collision** and refuses to repair it. |
| payload (`install.sh:744-749`) | `rm -rf "$CORT_HOME"` then `cp`. A hook firing in that window reads a missing or half-copied pack — and `extractor_version` over a short file list is a hash that looks legitimate. |
| `migrate_manifest_v2` (`install.sh:1083`) | It is the **first statement of `do_install`**, three lines above the comment promising preflight happens "before any mutation" (`:1087`). A preflight that aborts leaves the manifest migrated and nothing else installed. |

All four destroy the old state before the new one exists. Fixing them as four defects is four fixes; fixing the ordering is one principle applied four times, which is what this plan does.

**The fifth finding — no lock anywhere in `install.sh`** — is Task 5, because atomic publication of each artifact still lets two concurrent installers publish *different generations* of different artifacts.

## How crash-safety is tested without crashing, and what that test does *not* prove

Bash cannot be interrupted at a chosen instruction, so the tests observe the mechanism. Be precise
about how far that goes, because an earlier draft of this plan overclaimed it:

**A file replaced by rename gets a new inode; a file truncated in place keeps its inode.** That
discriminates reliably between exactly the two implementations at issue — rename cannot reuse the
old inode, because both files exist at the moment of the swap — and it is deterministic, with no
timing dependence.

**It is not a proof of atomicity.** `rm -f dest; cp tmp dest` has a window in which the file does
not exist, and it can produce a new inode. Worse, it can also produce the *same* one: measured on
this machine, `rm -f f; cp tmp f` reused inode 97814. So inode identity proves neither presence nor
absence of a gap — it only tells rename apart from truncate-in-place.

That is enough for these tasks, because the two implementations in question *are* rename and
truncate-in-place, and the review step for each task breaks it back to the truncating version and
watches the assertion go red. It is not enough to claim the code is crash-safe in general, and this
plan does not claim that.

**A seam is needed, and it is deliberate.** Each of these is one function; testing it through a
whole install would exercise preflight, ownership and mode dispatch instead. `install.sh` gains a
three-line sourcing guard (Task 1) whose only purpose is to let the smoke test call one publication
primitive. It carries no decision.

For the payload, the observable is the symlink: `$CORT_HOME` resolves to a complete generation
before and after the flip, because the flip is one rename of the link itself.

---

## File Structure

- `install.sh` — four publication sites and one lock. No new decisions.
- `tests/install-smoke.sh` — one test per property, each of which fails against today's implementation.

---

### Task 1: the manifest is replaced, never truncated

**Files:**
- Modify: `install.sh:313-322` (`record_manifest`)
- Test: `tests/install-smoke.sh`

**Interfaces:**
- Consumes: nothing.
- Produces: `record_manifest` with unchanged arguments and unchanged observable content; only its inode behaviour changes. Tasks 2-5 leave it alone.

- [ ] **Step 1: Write the failing test**

Add to `tests/install-smoke.sh`, after the manifest assertions in the first-install block:

```bash
# The manifest is the only record uninstall has of what exists, and record_manifest is called about
# nine times per run. Truncating the live file and appending afterwards means an interruption
# between the two loses the key. A file replaced by rename gets a new inode; a file truncated in
# place keeps its inode, so this is the mechanism itself rather than a proxy for it.
# `stat -c` is GNU; BSD and macOS use `stat -f`, and this installer supports Darwin
# (`install.sh:140-166`). Define the probe once, near the other assert helpers.
inode_of() { stat -c %i "$1" 2>/dev/null || stat -f %i "$1"; }

MANIFEST="$HOME/.local/share/cortexyoung/manifest"
before_ino="$(inode_of "$MANIFEST")"
before_body="$(cat "$MANIFEST")"
( SOURCE_ONLY=1; MANIFEST_FILE="$MANIFEST"; MANIFEST_DIR="$(dirname "$MANIFEST")"
  # shellcheck disable=SC1090
  . "$INSTALL_SH"
  record_manifest "smoke_probe" "value" )
after_ino="$(inode_of "$MANIFEST")"
if [ "$before_ino" != "$after_ino" ]; then
  pass "record_manifest replaces the manifest rather than truncating it"
else
  fail "record_manifest truncated the live manifest (inode unchanged: $before_ino)"
fi
assert_contains "$MANIFEST" "smoke_probe:value" "the probe key was recorded"
for k in cort_bin hook_settings manifest_version; do
  assert_contains "$MANIFEST" "$k:" "record_manifest kept $k while adding another"
done
```

`install.sh` has no sourcing mode, so add one. **Placement is the whole difficulty**: the guard must
run *after every function is defined* and *before the argument parser*, or it either returns before
`record_manifest` exists or falls through to the parser, which rejects unknown options with
`exit 2` (`install.sh:83`).

The constants are at the top (`install.sh:9-40`), the functions run from roughly `:100` to `:1075`,
and the parser is at `:64-85` — **above** the functions. So the guard cannot simply sit before the
parser. Put it immediately before the **mode dispatch** at the very bottom of the file (find it with
`grep -n 'MODE' install.sh | tail -3`), which is the first point at which every function is defined
and nothing has run:

```bash
# Sourcing seam: `source install.sh --source-only` leaves every function defined and returns before
# anything runs. Its only purpose is to let tests exercise one publication primitive rather than a
# whole install; it carries no decision and nothing in the normal path reads it.
if [ "${SOURCE_ONLY:-0}" = "1" ]; then
  return 0 2>/dev/null || true
fi
```

**An environment variable, not a flag**, because the parser at `:64-85` runs before the functions
and would reject `--source-only` with `exit 2` before the guard could see it. The test sets
`SOURCE_ONLY=1` and sources the file:

```bash
( SOURCE_ONLY=1; MANIFEST_FILE="$MANIFEST"; MANIFEST_DIR="$(dirname "$MANIFEST")"
  # shellcheck disable=SC1090
  . "$INSTALL_SH"
  record_manifest "smoke_probe" "value" )
```

Note the parser still runs when sourced — it sees no arguments, sets `MODE=install`, and stops.
That is harmless; the guard fires before anything acts on `MODE`.

- [ ] **Step 2: Run it to verify it fails**

Run: `bash tests/install-smoke.sh 2>&1 | grep -E "record_manifest|smoke_probe"`
Expected: `FAIL: record_manifest truncated the live manifest (inode unchanged: …)`.

- [ ] **Step 3: Write minimal implementation**

Replace `record_manifest` (`install.sh:313-322`):

```bash
record_manifest() {
  local key="$1" val="$2"
  mkdir -p "$MANIFEST_DIR"
  # Stage the whole next version, then swap it in with one rename. The old file is intact until the
  # rename and complete after it; there is no instant at which it is missing this key or any other.
  # It used to truncate the live file and append afterwards, which lost the key on any interruption
  # between the two -- from the only record uninstall has of what exists.
  local tmp; tmp="$(mktemp "$MANIFEST_DIR/.manifest.XXXXXX")"
  if [ -f "$MANIFEST_FILE" ]; then
    grep -v "^${key}:" "$MANIFEST_FILE" > "$tmp" 2>/dev/null || true
  fi
  echo "${key}:${val}" >> "$tmp"
  mv -f "$tmp" "$MANIFEST_FILE"
}
```

`mktemp` inside `$MANIFEST_DIR` rather than `$TMPDIR` is deliberate: `mv` is only atomic within one
filesystem, and a `/tmp` on a different mount would silently degrade to copy-then-unlink — the very
window this replaces.

- [ ] **Step 4: Run it to verify it passes**

Run: `bash tests/install-smoke.sh 2>&1 | grep -E "record_manifest|smoke_probe|kept"`
Expected: all PASS.

- [ ] **Step 5: Verify the test can actually fail**

Restore the old body (`cat "$tmp" > "$MANIFEST_FILE"; rm -f "$tmp"; echo … >> "$MANIFEST_FILE"`).
Expected: the inode assertion goes RED. Restore. If it stays green the probe is not reaching
`record_manifest` and the fixture must be fixed before continuing.

- [ ] **Step 6: Verify everything**

```bash
bash -n install.sh && bash -n tests/install-smoke.sh
bash tests/install-smoke.sh
cd rust && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
cd ../evals && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test --locked --all-targets
```
Expected: all exit 0.

- [ ] **Step 7: Commit**

```bash
git add install.sh tests/install-smoke.sh
git commit -m "fix(install): the manifest is replaced, never truncated

record_manifest truncated the live file and appended the key afterwards,
deleting its staged copy in between, about nine times per run. An
interruption there loses the key -- from the only record uninstall has of
what exists.

The staged file now carries the whole next version and one rename swaps it
in. It is staged inside MANIFEST_DIR rather than TMPDIR because mv is atomic
only within a filesystem, and a /tmp on another mount would degrade to
copy-then-unlink: the window this replaces."
```

---

### Task 2: a skill and its stamp are published as one unit

**Files:**
- Modify: `install.sh:209-219` (`write_skill`, `ensure_skill_stamp`)
- Test: `tests/install-smoke.sh`

**Interfaces:**
- Consumes: Task 1's `record_manifest` (unchanged usage).
- Produces: `write_skill "$src" "$dest"` with unchanged arguments; both the skill and its stamp are replaced by rename.

**Why the stamp matters more than it looks.** `skill_is_managed` answers "is this ours" by comparing the stamp's recorded SHA-256 against the file's actual hash (`install.sh:189-207`). A partially written skill hashes to something the stamp does not name, so the next run classifies it as an **unmanaged collision** and refuses to touch it without `--force`. A crash mid-write does not merely corrupt the file; it revokes the installer's own ownership of it.

- [ ] **Step 1: Write the failing test**

Use the same sourcing seam Task 1 added, and call `write_skill` directly.

An earlier draft drove this through a whole install and dirtied the deployed skill first. Both
halves were wrong. Dirtying it makes its hash stop matching its stamp, so `skill_is_managed`
(`install.sh:194-207`) reports an unmanaged collision and preflight **exits** (`install.sh:350-383`)
— `write_skill` is never reached, by either implementation. And the draft's assertion was
`inode changed OR content equals source`, which the current truncating implementation satisfies
through the second arm: it rewrites the correct bytes into the old inode. A test whose two arms let
the known-bad implementation through is worse than no test.

```bash
# A skill and its stamp are one unit: the stamp records the hash of the bytes beside it, and a
# mismatch is read as somebody else's file. Truncating the skill in place therefore does not merely
# corrupt it -- it revokes our own claim to it, and the next run refuses to repair what it no longer
# recognises as ours. Both must be replaced, so both inodes are checked.
SK_DIR="$(mktemp -d)"
printf 'seed\n' > "$SK_DIR/SKILL.md"
( SOURCE_ONLY=1; MANAGED_SIGNATURE="$MANAGED_SIGNATURE"
  # shellcheck disable=SC1090
  . "$INSTALL_SH"
  write_skill "$REPO_ROOT/skills/ast-grep/SKILL.md" "$SK_DIR/SKILL.md" )
sk_ino="$(inode_of "$SK_DIR/SKILL.md")"
st_ino="$(inode_of "$SK_DIR/$STAMP_NAME")"
( SOURCE_ONLY=1; MANAGED_SIGNATURE="$MANAGED_SIGNATURE"
  # shellcheck disable=SC1090
  . "$INSTALL_SH"
  write_skill "$REPO_ROOT/skills/ast-grep/SKILL.md" "$SK_DIR/SKILL.md" )
if [ "$sk_ino" != "$(inode_of "$SK_DIR/SKILL.md")" ]; then
  pass "the skill is published by replacement, not truncated in place"
else
  fail "the skill was truncated in place (inode unchanged: $sk_ino)"
fi
if [ "$st_ino" != "$(inode_of "$SK_DIR/$STAMP_NAME")" ]; then
  pass "the stamp is published by replacement too"
else
  fail "the stamp was truncated in place (inode unchanged: $st_ino)"
fi
assert_contains "$SK_DIR/$STAMP_NAME" "skill_sha256:" "the stamp still names the hash"
rm -rf "$SK_DIR"
```

The second `write_skill` writes identical bytes on purpose: the property is *how* it publishes, not
whether it noticed a change, and calling it twice removes any dependence on change detection
elsewhere. The stamp inode is checked because an earlier draft captured it and never used it, which
would have let an implementation rename the skill and keep truncating the stamp.

- [ ] **Step 2: Run it to verify it fails**

Run: `bash tests/install-smoke.sh 2>&1 | grep -E "truncated in place|by replacement|the stamp"`
Expected: **two** failures — the skill and the stamp are both truncated today.

- [ ] **Step 3: Write minimal implementation**

```bash
# ensure_skill_stamp: claim "$1", a SKILL.md we just wrote, in the stamp file beside it.
ensure_skill_stamp() {
  local stamp; stamp="$(skill_stamp_for "$1")"
  local tmp; tmp="$(mktemp "$(dirname "$stamp")/.stamp.XXXXXX")"
  printf '%s\nskill_sha256:%s\n' "$MANAGED_SIGNATURE" "$(skill_hash "$1")" > "$tmp"
  mv -f "$tmp" "$stamp"
}

# write_skill: publish "$1" to "$2" byte-for-byte, then claim it. Nothing is inserted into the
# document -- what lands in an agent home directory is exactly what is in skills/<name>/SKILL.md.
#
# Staged and renamed, because the stamp records the hash of the bytes beside it: a half-written
# skill hashes to something the stamp does not name, and `skill_is_managed` then reads our own file
# as somebody else's and refuses to repair it.
write_skill() {
  local tmp; tmp="$(mktemp "$(dirname "$2")/.skill.XXXXXX")"
  cat "$1" > "$tmp"
  mv -f "$tmp" "$2"
  ensure_skill_stamp "$2"
}
```

The stamp is still written after the skill, and that ordering is correct: the stamp names the hash
of bytes that must already be in place. The window that remains — skill new, stamp old — is
recoverable, because the next run sees a hash the stamp does not name, reports the collision, and
`--force` adopts it. The window this removes was not recoverable: a *partial* skill is not a
document either loader can read.

- [ ] **Step 4: Run it to verify it passes**

Run: `bash tests/install-smoke.sh 2>&1 | grep -E "truncated in place|by replacement"`
Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Restore `cat "$1" > "$2"` in `write_skill`. Expected: RED. Restore.

- [ ] **Step 6: Verify everything**

Same six commands as Task 1 Step 6. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add install.sh tests/install-smoke.sh
git commit -m "fix(install): a skill and its stamp are published as one unit

write_skill truncated the live SKILL.md and stamped it afterwards. The stamp
records the hash of the bytes beside it, so a half-written skill hashes to
something the stamp does not name -- and skill_is_managed then reads our own
file as an unmanaged collision and refuses to repair it without --force. A
crash did not merely corrupt the file; it revoked our claim to it.

Both are staged and renamed now. The remaining window -- new skill, old stamp
-- is recoverable by design: the next run reports the collision and --force
adopts it. A partial document is not."
```

---

### Task 3: the payload is a generation, activated by one symlink flip

**Files:**
- Modify: `install.sh:744-762` (payload publication), and the uninstall path that removes `$CORT_HOME`
- Test: `tests/install-smoke.sh`

**Interfaces:**
- Consumes: Task 1's `record_manifest`.
- Produces: `$CORT_HOME` is a **symlink** to `$MANIFEST_DIR/cort-<sha>`, where `<sha>` is the first 12 hex characters of the built binary's SHA-256. The shim's `CORT_PACK_DIR="$CORT_HOME/pack"` and `exec "$CORT_HOME/cort"` are unchanged, because a symlink resolves at open time.

**Why a symlink and not a directory rename.** `rename(2)` cannot replace a non-empty directory, so swapping `$CORT_HOME` in place is two renames with a gap in between — and `$CORT_HOME/pack` does not exist during it. A hook firing there reads a pack that is missing or half-copied, and `extractor_version` over a shortened file list is a hash that looks perfectly legitimate (`pack.rs`; the walk now returns that failure, but the half-copied case still hashes fewer files). Renaming a **symlink** over an existing symlink *is* atomic, which is why Homebrew's `opt/`, Nix and Capistrano all take this shape.

- [ ] **Step 1: Write the failing test**

Add to `tests/install-smoke.sh` in the first-install block:

```bash
# rename(2) cannot replace a non-empty directory, so swapping CORT_HOME in place is two renames
# with a gap where $CORT_HOME/pack does not exist. A hook firing in that gap reads a missing or
# half-copied pack, and a hash over a shortened file list looks perfectly legitimate. Renaming a
# symlink over a symlink is atomic; that is the whole mechanism.
CORT_HOME_PATH="$HOME/.local/share/cortexyoung/cort"
if [ -L "$CORT_HOME_PATH" ]; then
  pass "CORT_HOME is a symlink to a generation"
else
  fail "CORT_HOME is a directory, so activation cannot be one rename"
fi
# `readlink` exits 1 on a plain directory, which is fatal under the harness's `set -e` -- so on the
# RED run this line would abort the suite instead of letting the assertion above be reported.
gen1="$(readlink "$CORT_HOME_PATH" 2>/dev/null || echo "NOT-A-LINK")"
assert_file_exists "$CORT_HOME_PATH/cort" "the active generation carries the binary"
assert_file_exists "$CORT_HOME_PATH/pack/sgconfig.yml" "the active generation carries the pack"

# A second install must leave a usable installation. `|| true` here would let "the install failed
# before touching anything" pass every assertion below, because the first installation is still
# valid -- so its status is asserted rather than discarded.
if bash "$INSTALL_SH" >/tmp/smoke_reinstall.log 2>&1; then
  pass "an identical reinstall succeeds"
else
  fail "the reinstall failed"; sed 's/^/    /' /tmp/smoke_reinstall.log | tail -5
fi
gen2="$(readlink "$CORT_HOME_PATH" 2>/dev/null || echo "NOT-A-LINK")"
if [ "$gen1" != "NOT-A-LINK" ] && [ "$gen2" != "NOT-A-LINK" ]; then
  pass "both generations are named"
else
  fail "a generation link was missing (gen1=$gen1 gen2=$gen2)"
fi
assert_file_exists "$CORT_HOME_PATH/cort" "the binary survived the reinstall"
assert_file_exists "$CORT_HOME_PATH/pack/sgconfig.yml" "the pack survived the reinstall"
# The reinstall is byte-identical, so it lands on the same generation id. That is the case an
# earlier draft got wrong: it did `rm -rf "$gen_dir"` while the live link still pointed there, so a
# failure between the removal and the move left the shim pointing at nothing.
if [ "$gen1" = "$gen2" ]; then
  pass "an identical rebuild reuses its generation rather than deleting and recreating it"
else
  fail "an identical rebuild changed generation (gen1=$gen1 gen2=$gen2)"
fi
```

- [ ] **Step 2: Run it to verify it fails**

Run: `bash tests/install-smoke.sh 2>&1 | grep -E "CORT_HOME is"`
Expected: `FAIL: CORT_HOME is a directory, so activation cannot be one rename`.

- [ ] **Step 3: Write minimal implementation**

Replace the payload block (`install.sh:744-749`):

```bash
  # Stage the whole generation, validate it, then activate with one rename of the symlink. Nothing
  # ever observes a partial CORT_HOME: the link points at the old generation until the instant it
  # points at the new one. The generation is named by the binary's own hash, so an identical
  # rebuild reuses it and a changed one cannot collide with it.
  # The generation is named by the binary AND the pack, because the pack is read from the runtime
  # directory rather than compiled in (`rust/src/pack.rs`): a pack-only change keeps the binary hash
  # identical, and naming the generation after the binary alone would make two different payloads
  # collide on one name.
  local gen_id gen_dir staging
  gen_id="$(payload_id "$crate_bin" "$SCRIPT_DIR/src/pack")"
  gen_dir="$MANIFEST_DIR/cort-$gen_id"
  staging="$MANIFEST_DIR/.cort-staging.$$"

  rm -rf "$staging"
  mkdir -p "$staging"
  cp "$crate_bin" "$staging/cort"
  cp -R "$SCRIPT_DIR/src/pack" "$staging/pack"
  chmod 755 "$staging/cort"

  # Validate before activating. A generation missing part of its pack would stamp indexes with a
  # hash over a shortened file list, which is the failure that looks most like success -- so the
  # check is the pack's own identity, not a file count. A count passes a staged pack with one rule
  # deleted and one substituted.
  [ -x "$staging/cort" ] || die "staged generation has no executable: $staging/cort"
  [ -f "$staging/pack/sgconfig.yml" ] || die "staged generation has no pack: $staging/pack"
  local staged_id source_id
  staged_id="$(payload_id "$staging/cort" "$staging/pack")"
  source_id="$(payload_id "$crate_bin" "$SCRIPT_DIR/src/pack")"
  if [ "$staged_id" != "$source_id" ]; then
    rm -rf "$staging"
    die "staged payload does not match its source ($staged_id != $source_id)"
  fi

  # Promote. The live generation is NEVER removed: on an identical rebuild `$gen_dir` already exists
  # and the symlink already points at it, so deleting it to make room would leave the installed shim
  # pointing at nothing if anything failed before the move. An existing generation with this id has
  # the same content by construction -- that is what the id means -- so it is reused.
  if [ -d "$gen_dir" ]; then
    rm -rf "$staging"
  else
    mv "$staging" "$gen_dir"
  fi

  # A pre-symlink installation has a real directory here and no generation to fall back to. This is
  # the one moment that cannot be made atomic, and it happens once per machine.
  if [ -d "$CORT_HOME" ] && [ ! -L "$CORT_HOME" ]; then
    rm -rf "$CORT_HOME"
  fi
  swap_symlink "$gen_dir" "$CORT_HOME"
```

Two helpers, beside the other file primitives:

```bash
# payload_id: one name for a binary plus a pack. The pack is read at runtime, so a pack-only change
# must produce a different generation even though the binary is byte-identical.
payload_id() {
  local bin="$1" pack="$2"
  { skill_hash "$bin"
    find "$pack" -name '*.yml' | sort | while IFS= read -r f; do skill_hash "$f"; done
  } | skill_hash /dev/stdin | cut -c1-12
}

# swap_symlink: point "$2" at "$1" with one rename, replacing an existing symlink rather than
# following it into its target. GNU mv spells that `-T`; BSD and macOS spell it `-h`, and this
# installer supports Darwin (`install.sh:140-166`). Plain `mv` would move the new link *inside* the
# directory the old one points at.
swap_symlink() {
  local target="$1" link="$2" tmp
  tmp="$(dirname "$link")/.cort-link.$$"
  rm -f "$tmp"
  ln -s "$target" "$tmp"
  if mv -T "$tmp" "$link" 2>/dev/null; then return 0; fi
  if mv -h "$tmp" "$link" 2>/dev/null; then return 0; fi
  rm -f "$tmp"
  die "no portable atomic symlink swap on this platform (need mv -T or mv -h)"
}
```

`skill_hash /dev/stdin` reads the concatenated hashes rather than a file list, so a renamed rule
file changes the id. Verify `skill_hash` accepts a stream on this platform before relying on it
(`sha256sum /dev/stdin` and `shasum -a 256 /dev/stdin` both do); if it does not, write the list to
a `mktemp` file and hash that.

`skill_hash` (`install.sh:236-238`) already is "SHA-256 of a file, or empty", despite its name —
it is used for skills today but names no skill in its body. Reuse it rather than adding a second
hasher; if its name grates, rename it in this task and update its three call sites, but do **not**
write a parallel one. A second copy of "hash a file" is how this repository got two homes for the
ast-grep pin.

Superseded generations are kept, deliberately: one costs ~15 MB and is the only thing a rollback
could use. Cleanup belongs to plan 3c's upgrader, which will know which generation is current.

**But uninstall must stop leaking them.** Both uninstall paths do
`if [ -d "$CORT_HOME" ]; then rm -rf "$CORT_HOME"; fi` (`install.sh:981-984` and `:1063-1066`).
Against a symlink, `-d` follows it and succeeds, and `rm -rf` then removes **only the link** — every
`cort-*` generation stays behind forever. Replace both with:

```bash
    if [ -L "$CORT_HOME" ] || [ -d "$CORT_HOME" ]; then
      rm -rf "$CORT_HOME"
      # The link is gone; the generations it and its predecessors named are not. Remove only
      # directories this installer names, inside the directory it owns -- never whatever a
      # user-created symlink happened to point at.
      find "$MANIFEST_DIR" -maxdepth 1 -type d -name 'cort-*' -exec rm -rf {} +
      info "removed $CORT_HOME and its generations"
    fi
```

The glob is deliberately narrow. The no-manifest branch (`:1063`) is documented as conservative and
has no ownership record, so it may remove `cort-*` under `$MANIFEST_DIR` — a path this installer
created — but must never follow the link to decide what to delete.

Then run `grep -n 'CORT_HOME' install.sh` and check every remaining reader against the new type. The
shim embeds `$CORT_HOME` as an absolute string (`:753-756`), which is exactly what makes the flip
take effect for already-installed shims: the path is resolved at exec time, not at write time.

- [ ] **Step 4: Run it to verify it passes**

Run: `bash tests/install-smoke.sh 2>&1 | grep -E "CORT_HOME is|generation"`
Expected: all PASS.

- [ ] **Step 5: Verify the test can actually fail**

Replace the flip with the old `rm -rf "$CORT_HOME"; mkdir -p "$CORT_HOME"; cp …`. Expected: the
symlink assertion goes RED. Restore.

Two more breaks that matter more than the first:

2. Delete one `.yml` from the staged pack just before the validation
   (`rm "$staging/pack/rules/rust.yml"`). Expected: the install **dies** with
   `staged payload does not match its source`. That assertion stands between a half-copied pack and
   an index stamped with a plausible-looking hash.
3. **Substitute** rather than delete: copy one rule file over another
   (`cp "$staging/pack/rules/rust.yml" "$staging/pack/rules/python.yml"`). Expected: it still dies.
   A file *count* would pass this, which is why the check is the pack's identity.

- [ ] **Step 6: Verify everything**

Same six commands as Task 1 Step 6, plus:
```bash
./install.sh --check
readlink "$HOME/.local/share/cortexyoung/cort"
cort --version
```
Expected: `--check` reports `check: OK`, the link resolves to a `cort-<12 hex>` directory, and the
shim still runs.

- [ ] **Step 7: Commit**

```bash
git add install.sh tests/install-smoke.sh
git commit -m "fix(install): the payload is a generation, activated by one symlink flip

rm -rf CORT_HOME followed by cp left a window in which the pack was missing or
half-copied, and a hash over a shortened file list is one that looks
perfectly legitimate -- stamped into an index it never matches again and
never explains itself.

rename(2) cannot replace a non-empty directory, so swapping the directory in
place would be two renames with the same gap between them. Renaming a symlink
over a symlink is atomic, which is the shape Homebrew's opt, Nix and
Capistrano all take. The generation is named by the binary's own hash, and it
is validated -- executable present, pack present, rule count equal to source
-- before anything points at it."
```

---

### Task 4: nothing is mutated before the preflight that promises to precede mutation

**Files:**
- Modify: `install.sh:1080-1095` (`do_install`'s opening)
- Test: `tests/install-smoke.sh`

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: no signature change; `migrate_manifest_v2` runs after every `preflight_skill_at`.

- [ ] **Step 1: Write the failing test**

```bash
# The comment at install.sh:1087 promises preflight runs "before any mutation". migrate_manifest_v2
# ran four lines above it, so an install that aborts in preflight left the manifest migrated and
# nothing else installed -- the first step of an upgrade violating its own ordering rule.
TMPHOME="$(mktemp -d)"; export HOME="$TMPHOME"
mkdir -p "$HOME/.local/share/cortexyoung"
printf 'xg_bin:/old/xg\nskill:/old/skill\n' > "$HOME/.local/share/cortexyoung/manifest"
# An unreadable skill source is what preflight refuses on.
mkdir -p "$HOME/.claude/skills/ast-grep"
printf 'not a skill, no frontmatter\n' > "$HOME/.claude/skills/ast-grep/SKILL.md"
bash "$INSTALL_SH" >/tmp/smoke_preflight.log 2>&1 || true
if grep -q '^manifest_version:2' "$HOME/.local/share/cortexyoung/manifest"; then
  fail "the manifest was migrated before preflight decided whether to proceed"
else
  pass "preflight runs before the manifest is touched"
fi
```

`TMPHOME` is `mktemp -d` at `tests/install-smoke.sh:73` and every later block reassigns `HOME` from
it, so allocate a fresh one rather than clearing the shared one — clearing it would strand the
blocks that run after this test.

Read the existing unmanaged-collision block for the exact shape that makes preflight refuse; the
fixture above must actually abort the install, or the test proves nothing. Verify that first by
checking the log: `grep -q 'UNMANAGED\|collision' /tmp/smoke_preflight.log`. If preflight adopts
rather than refuses, use the shape that block already uses.

- [ ] **Step 2: Run it to verify it fails**

Expected: `FAIL: the manifest was migrated before preflight decided whether to proceed`.

- [ ] **Step 3: Write minimal implementation**

`migrate_manifest_v2` is the first statement of `do_install` (`install.sh:1083`). Move that one
line to immediately **after** the last `preflight_skill_at` — read the block to find it, because
the xgrep preflight is inside an `if` and the call must land after the whole group, not inside it.
Leave the comment where it is; it becomes true rather than aspirational.

`detect_platform` and `resolve_bin_dir` stay where they are: neither mutates anything, and
`resolve_bin_dir` is what the preflights need in order to know where they are preflighting.

- [ ] **Step 4: Run it to verify it passes**

Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Move the call back above the preflights. Expected: RED. Restore.

- [ ] **Step 6: Verify everything**

Same six commands as Task 1 Step 6. All exit 0. Watch the existing manifest-v2 migration test in
`tests/install-smoke.sh` — it must stay green without edits, because migration still happens on
every successful install, just later.

- [ ] **Step 7: Commit**

```bash
git add install.sh tests/install-smoke.sh
git commit -m "fix(install): preflight runs before the manifest is touched

The comment promising preflight happens before any mutation sat four lines
below the migration it was describing. An install that aborted in preflight
left the manifest migrated and its version advanced while nothing else was
installed -- the first step of an upgrade breaking its own ordering rule."
```

---

### Task 5: two installers cannot interleave

**Files:**
- Modify: `install.sh:1080` (`do_install`'s opening) and the uninstall mode
- Test: `tests/install-smoke.sh`

**Interfaces:**
- Consumes: Tasks 1-4.
- Produces: the mutating modes hold an exclusive `flock` on `$MANIFEST_DIR/.install.lock` for their whole run. `--check` does not take it.

**Why a lock is still needed after Tasks 1-3.** Each artifact is now published atomically, but two installers running concurrently can still publish *different generations of different artifacts*: one flips the payload symlink to generation A while the other writes a manifest naming generation B, and each of the read-modify-write hook updates is a separate atomic rename with no ordering between them. Atomicity per file is not atomicity per install.

- [ ] **Step 1: Write the failing test**

```bash
# Per-file atomicity is not per-install atomicity: two installers can publish different generations
# of different artifacts. The lock is held for the whole mutating run, so the second waits rather
# than interleaving.
LOCK="$HOME/.local/share/cortexyoung/.install.lock"
# A sleep is not an acquisition handshake: under load the installer can take the lock before the
# background subshell reaches flock, and the test then fails for a reason that has nothing to do
# with the code. The holder signals through a sentinel instead.
ACQUIRED="$(mktemp -u)"
( flock -x 9; : > "$ACQUIRED"; sleep 5 ) 9>"$LOCK" &
holder=$!
for _ in $(seq 1 100); do
  if [ -e "$ACQUIRED" ]; then break; fi
  sleep 0.05
done
if [ ! -e "$ACQUIRED" ]; then
  fail "the probe never acquired the lock; the timing result below would be meaningless"
fi
start=$(date +%s)
timeout 20 bash "$INSTALL_SH" >/dev/null 2>&1 || true
elapsed=$(( $(date +%s) - start ))
wait "$holder" 2>/dev/null || true
rm -f "$ACQUIRED"
if [ "$elapsed" -ge 4 ]; then
  pass "a second installer waits for the first rather than interleaving"
else
  fail "the installer ran while another held the lock (${elapsed}s)"
fi
```

`flock` is util-linux and is not present everywhere. Guard the test:

```bash
if ! command -v flock >/dev/null 2>&1; then
  echo "  SKIP: flock unavailable"
else
  # … the block above …
fi
```

- [ ] **Step 2: Run it to verify it fails**

Expected: `FAIL: the installer ran while another held the lock (0s)`.

- [ ] **Step 3: Write minimal implementation**

As the first statement of `do_install` (`install.sh:1080-1083`), before `detect_platform`. Do the
same in the uninstall mode, which mutates the same paths — find it with `grep -n 'uninstall complete' install.sh`
and put it at the top of that function:

```bash
  # One installer at a time. Every artifact below is published atomically on its own, but two
  # concurrent runs can still leave a manifest naming one generation while the symlink points at
  # another: atomicity per file is not atomicity per install. The lock is advisory and only
  # cooperating installers honour it, which is all of them.
  #
  # `--check` deliberately does not take it: a report must not be blocked by a running install, and
  # it mutates nothing.
  mkdir -p "$MANIFEST_DIR"
  if command -v flock >/dev/null 2>&1; then
    exec 9>"$MANIFEST_DIR/.install.lock"
    flock -x 9 || die "another installer holds the lock and would not yield"
  fi
```

`exec 9>` keeps the descriptor open for the life of the script, so the lock is released by the
kernel when the process exits — including on a crash. Nothing has to clean it up, and a stale lock
file is not a stale lock.

- [ ] **Step 4: Run it to verify it passes**

Expected: PASS.

- [ ] **Step 5: Verify the test can actually fail**

Remove the `flock -x 9` line (keep the `exec 9>`). Expected: RED with `(0s)`. Restore.

Second break: run `./install.sh --check` while the probe holds the lock. Expected: it completes
immediately — a report must not be blocked by a running install.

- [ ] **Step 6: Verify everything**

Same six commands as Task 1 Step 6. All exit 0.

- [ ] **Step 7: Commit**

```bash
git add install.sh tests/install-smoke.sh
git commit -m "feat(install): one installer at a time

Every artifact is published atomically now, but two concurrent runs could
still leave a manifest naming one generation while the symlink points at
another. Atomicity per file is not atomicity per install.

The descriptor is held for the life of the script, so the kernel releases the
lock when the process exits, crash included -- a stale lock file is not a
stale lock. --check does not take it: a report must not be blocked by a
running install, and it mutates nothing."
```

---

## Self-Review

**Spec coverage.** Implements spec §3(a)'s activation mechanism, §5's stage → validate → activate
→ cleanup ordering as it applies to the installer's own publication, and four of the five findings
verified in §9 (items 1, 2, 4, 5). §9's item 3 — `--status`'s `wired` being a token test on the
command string rather than a comparison against the canonical entry — is **not** here: it is a
diagnosis defect, not a publication one, and it belongs with 3c's component diagnosis.

**Not in this plan:** the shim template, ast-grep pin and manifest key-set moving to a single home
(3b); `cort-upgrade` itself, the two flocks that coordinate with *hooks* rather than with other
installers, the verdict taxonomy and escape hatches, and index migration (3c); removing the unread
`projects.extractor_version` column and making `ensure_schema`'s `graph_pending` conditional (3c,
because the column's removal is only worth a migration that does not charge every index a rebuild).

**Placeholders:** none. Two steps deliberately send the implementer to read existing code rather
than quoting it — Task 3's reuse of `skill_hash` and Task 4's refusable-state fixture — because both
already exist in the file and a second copy is the mistake this project keeps paying for. Both say
exactly what to search for.

**Type consistency:** `$CORT_HOME` remains the path every other line already uses; only its type
changes, from directory to symlink, and the shim resolves it at open time either way.

## What the review changed

One round; every finding verified against source or measured before being accepted. Two of them
corrected reasoning of mine rather than code.

**The plan would have deleted the live generation on an identical reinstall.** `gen_id` was the
binary's hash, `$CORT_HOME` already pointed at `$gen_dir`, and the implementation did
`rm -rf "$gen_dir"` to make room for the staged copy. A failure between that and the `mv` left the
installed shim pointing at nothing — in the task whose entire purpose is removing exactly that
window. An existing generation with this id has this content by construction, so it is now reused
and never removed. The id also covers the pack, because the pack is read at runtime rather than
compiled in: a pack-only change keeps the binary hash identical and would have collided on one name.

**The inode test was overclaimed, and measurement made it worse than the reviewer said.** The plan
called it "not a proxy — it *is* atomic replacement". It is not: `rm -f dest; cp tmp dest` has a
window and can produce a new inode. Measured here, that sequence reused inode 97814 — so inode
identity proves neither the presence nor the absence of a gap. It discriminates rename from
truncate-in-place, which is what these tasks need, and the plan now says only that.

**Three tests would have aborted or passed for the wrong reason.** The sourcing guard sat below the
constants, so it returned before `record_manifest` was defined and the probe was "command not
found" — fatal under the harness's `set -e`; it is now an environment variable checked immediately
before the mode dispatch, because the argument parser runs above the function definitions and would
reject a flag with `exit 2`. Task 2 dirtied the deployed skill first, which makes preflight refuse
the whole install so `write_skill` is never reached — and its assertion was
`inode changed OR content equals source`, whose second arm the truncating implementation satisfies.
Task 3 called `readlink` unguarded on what is a plain directory during the RED run, and `readlink`
exits 1 there, aborting the suite before the assertion could be reported.

**Portability, on a platform this installer supports.** `mv -T` is a GNU extension; BSD and macOS
spell it `-h`. `stat -c` is GNU; BSD is `stat -f`. Both are now detected. `install.sh:140-166`
handles Darwin explicitly, so neither was theoretical.

**Validation was a file count.** A staged pack with one rule deleted and one substituted has the
same count as its source. It is now the pack's own identity, and Step 5 breaks it both ways.

**Uninstall leaked every generation.** `[ -d "$CORT_HOME" ]` follows a symlink and succeeds, and
`rm -rf` then removes only the link. Both uninstall paths now remove the generations too, matching
`cort-*` inside the directory this installer owns rather than following the link to decide.

**One finding is recorded rather than fixed.** A process that has already exec'd the old binary
holds `CORT_PACK_DIR` as the textual link path, so after a flip its later pack reads resolve through
the new link: it can run an old binary against a new pack. Making the shim resolve the generation
once and pass the resolved path would fix it, but the shim template moves to Rust in plan 3b and
changing it twice is worse than changing it once. **This is strictly better than today**, where the
same process reads a pack that is being `rm -rf`'d out from under it — but it is not zero, and plan
3b owns closing it.
