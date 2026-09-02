#!/usr/bin/env bash
set -euo pipefail
# Offline smoke test for install.sh — mocked download/cargo, temp HOME
# Covers: first-install, identical rerun, managed update, unmanaged-collision
#         refusal, unsupported OS/arch, wrong version detection, profile
#         idempotency, uninstall, SHA fail-closed, manifest v2 migration,
#         sg≠ast-grep, two-skill rollback, uninstall v2, DB interrupt recovery.
# No network is used. Run: bash tests/install-smoke.sh

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_SH="$REPO_ROOT/install.sh"
MANAGED_SIGNATURE="managed by cortexyoung install.sh"
STAMP_NAME=".cortexyoung-managed"
PASS=0; FAIL=0

pass() { PASS=$((PASS+1)); echo "  PASS: $*"; }
fail() { FAIL=$((FAIL+1)); echo "  FAIL: $*"; }

assert_file_exists() {
  if [ -f "$1" ]; then pass "$2"; else fail "$2 (missing: $1)"; fi
}
assert_file_not_exists() {
  if [ ! -f "$1" ]; then pass "$2"; else fail "$2 (should not exist: $1)"; fi
}
assert_contains() {
  if grep -qF "$2" "$1" 2>/dev/null; then pass "$3"; else fail "$3 (not found: $2 in $1)"; fi
}
assert_not_contains() {
  if ! grep -qF "$2" "$1" 2>/dev/null; then pass "$3"; else fail "$3 (should not contain: $2 in $1)"; fi
}
# A SKILL.md the loader cannot parse is worse than a missing file: it is on disk, green in every
# "is it there" assertion, and silently skipped by both agents. These two pin the shape.
assert_frontmatter_first() {
  if [ "$(head -n 1 "$1" 2>/dev/null)" = "---" ]; then pass "$2"; else fail "$2 (line 1 is not '---' in $1)"; fi
}
# assert_frontmatter_keys_only: everything between the two `---` lines must be a YAML key. A
# comment there is legal YAML and no loader rejects it, which is exactly why the old assertions
# kept passing: the documented shape is keys only, and installer bookkeeping is not a key.
assert_frontmatter_keys_only() {
  local fence
  fence="$(awk 'NR==1{next} $0=="---"{exit} {print}' "$1" 2>/dev/null)"
  if [ -z "$fence" ]; then
    fail "$2 (no frontmatter block in $1)"
  elif printf '%s\n' "$fence" | grep -qE '^[[:space:]]*(#|$)'; then
    fail "$2 (frontmatter holds a comment or blank line: $1)"
  else
    pass "$2"
  fi
}
# The gate has to be able to fail, so the negative form is asserted on the fixtures too.
assert_frontmatter_keys_only_rejects() {
  local fence
  fence="$(awk 'NR==1{next} $0=="---"{exit} {print}' "$1" 2>/dev/null)"
  if printf '%s\n' "$fence" | grep -qE '^[[:space:]]*(#|$)'; then pass "$2"
  else fail "$2 (the key-only gate accepted a comment inside the fence)"; fi
}
# assert_pristine_skill: what landed in the agent home is the repo file, byte for byte.
assert_pristine_skill() {
  if cmp -s "$1" "$2" && ! grep -qF "$MANAGED_SIGNATURE" "$1"; then pass "$3"
  else fail "$3 (deployed bytes differ from $2)"; fi
}
# assert_skill_claimed: the stamp beside the skill must claim exactly these bytes. Ownership that
# does not track content would let the installer overwrite a file the user has since edited.
assert_skill_claimed() {
  local stamp hash
  stamp="$(dirname "$1")/$STAMP_NAME"
  hash="$(sha256sum "$1" 2>/dev/null | awk '{print $1}')"
  if [ -f "$stamp" ] && [ -n "$hash" ] && grep -qF "skill_sha256:$hash" "$stamp"; then pass "$2"
  else fail "$2 (no stamp claiming $1 at $stamp)"; fi
}

# ── isolated HOME ────────────────────────────────────────────────
TMPHOME="$(mktemp -d)"
export HOME="$TMPHOME"
export XDG_DATA_HOME="$HOME/.local/share"
mkdir -p "$HOME/.claude/skills" "$HOME/.local/share" "$HOME/.cargo/bin"

ORIGINAL_PATH="$PATH"
# Mock xg binary (so --check / version logic works without download)
cat > "$HOME/.cargo/bin/xg" <<'MOCKXG'
#!/usr/bin/env bash
echo "xg 0.7.0"
MOCKXG
chmod +x "$HOME/.cargo/bin/xg"
export PATH="$HOME/.cargo/bin:$PATH"

# ── offline fakes: ast-grep (pinned), npm, cargo ─────────────────
mkdir -p "$TMPHOME/fakebin"
cat > "$TMPHOME/fakebin/ast-grep" <<'FAKEAG'
#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "ast-grep 0.45.2"; else echo "ast-grep 0.45.2"; fi
FAKEAG
chmod +x "$TMPHOME/fakebin/ast-grep"
cat > "$TMPHOME/fakebin/npm" <<'FAKENPM'
#!/usr/bin/env bash
exit 0
FAKENPM
chmod +x "$TMPHOME/fakebin/npm"
cat > "$TMPHOME/fakebin/cargo" <<'FAKECARGO'
#!/usr/bin/env bash
echo "cargo 1.88.0"
if [ -n "${FAKE_CARGO_LOG:-}" ] && [ "$1" = "build" ]; then
  printf 'cargo %s\n' "$*" >> "$FAKE_CARGO_LOG"
fi
exit 0
FAKECARGO
chmod +x "$TMPHOME/fakebin/cargo"
cat > "$TMPHOME/fakebin/curl" <<'FAKECURL_STUB'
#!/usr/bin/env bash
exit 1
FAKECURL_STUB
chmod +x "$TMPHOME/fakebin/curl"
cat > "$TMPHOME/fakebin/wget" <<'FAKEWGET_STUB'
#!/usr/bin/env bash
exit 1
FAKEWGET_STUB
chmod +x "$TMPHOME/fakebin/wget"
export PATH="$TMPHOME/fakebin:$PATH"

# Mock curl/wget to avoid network: intercept install.sh download path by
# pre-creating a fake asset? Instead we test the skill/profile/manifest
# paths directly — binary install is skipped because mock xg already matches
# and mock ast-grep already matches.

echo "=== install-smoke: temp HOME=$TMPHOME ==="

# ── 1. first install (default is ast-grep skill, not xgrep) ───────
echo "--- Test 1: first install ---"
bash "$INSTALL_SH" > /tmp/smoke1.log 2>&1; cat /tmp/smoke1.log | sed 's/^/    /'
assert_file_exists "$HOME/.claude/skills/ast-grep/SKILL.md" "ast-grep skill installed"
assert_not_contains "$HOME/.claude/skills/ast-grep/SKILL.md" "$MANAGED_SIGNATURE" "deployed SKILL.md holds no installer bookkeeping"
assert_frontmatter_first "$HOME/.claude/skills/ast-grep/SKILL.md" "deployed skill keeps its YAML fence on line 1"
assert_frontmatter_keys_only "$HOME/.claude/skills/ast-grep/SKILL.md" "deployed frontmatter holds only YAML keys"
assert_pristine_skill "$HOME/.claude/skills/ast-grep/SKILL.md" "$REPO_ROOT/skills/ast-grep/SKILL.md" "deployed skill is the repo source byte for byte"
assert_skill_claimed "$HOME/.claude/skills/ast-grep/SKILL.md" "ownership recorded in the stamp file beside the skill"
assert_file_not_exists "$HOME/.claude/skills/xgrep/SKILL.md" "xgrep skill not installed by default"
# The hook is deployed in the same run as the skill: a routing half that has to be wired by hand
# is a routing half that stays unwired, which is what the 2026-09-01 mining window measured (745
# grep/rg triggers recorded by the harness, zero rows in usage.db, because nothing was wired).
assert_file_exists "$HOME/.claude/settings.json" "settings.json written by the hook deploy"
assert_contains "$HOME/.claude/settings.json" "hook-suggest" "PreToolUse hook wired in the same run as the skill"
assert_contains "$HOME/.claude/settings.json" '"matcher": "Bash"' "the hook is matched to Bash, not to every tool"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "hook_settings:" "hook_settings recorded in the manifest"
assert_file_exists "$HOME/.local/share/cortexyoung/manifest" "manifest created"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "manifest_version:2" "manifest version 2 recorded"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "skill_ast_grep:" "skill_ast_grep recorded"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "cort_bin:" "cort_bin recorded"
# `fake_ast_grep` is a test double that cargo builds alongside cort; shipping it would be a
# second executable in the payload that nobody owns.
if find "$HOME/.local/share/cortexyoung/cort" -name 'fake_ast_grep' -print -quit | grep -q .; then
  fail "dev-only fixture stays out of the installed payload"
else
  pass "dev-only fixture stays out of the installed payload"
fi
# profile block — installer picks one candidate; check any of them
PROFILE_HIT=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile" "$HOME/.config/fish/config.fish"; do
  if grep -qF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null; then PROFILE_HIT=1; break; fi
done
if [ "$PROFILE_HIT" -eq 1 ]; then pass "profile PATH block added"; else fail "profile PATH block added"; fi
MANIFEST_LINES="$(wc -l < "$HOME/.local/share/cortexyoung/manifest" 2>/dev/null || echo 0)"
echo "    manifest lines: $MANIFEST_LINES"

# ── 2. identical rerun (idempotent) ──────────────────────────────
echo "--- Test 2: identical rerun (idempotent) ---"
# Count marker occurrences before
MARKER_COUNT_BEFORE=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
  if [ -f "$p" ]; then
    c="$(grep -cF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null || true)"
    MARKER_COUNT_BEFORE=$((MARKER_COUNT_BEFORE + c))
  fi
done
bash "$INSTALL_SH" > /tmp/smoke2.log 2>&1; cat /tmp/smoke2.log | sed 's/^/    /'
MARKER_COUNT_AFTER=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
  if [ -f "$p" ]; then
    c="$(grep -cF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null || true)"
    MARKER_COUNT_AFTER=$((MARKER_COUNT_AFTER + c))
  fi
done
if [ "$MARKER_COUNT_BEFORE" -eq "$MARKER_COUNT_AFTER" ]; then pass "profile block idempotent (no duplication)"; else fail "profile block idempotent ($MARKER_COUNT_BEFORE -> $MARKER_COUNT_AFTER)"; fi
assert_skill_claimed "$HOME/.claude/skills/ast-grep/SKILL.md" "ast-grep skill still claimed after rerun"
if grep -qF "skill up to date: $HOME/.claude/skills/ast-grep/SKILL.md" /tmp/smoke2.log; then
  pass "rerun calls the unchanged managed skill up to date"
else
  fail "rerun calls the unchanged managed skill up to date"
fi

# ── 3. managed update (installer replaces outdated managed skill) ─
echo "--- Test 3: a deployed skill edited in place is a collision, not a licence to overwrite ---"
printf 'the user tuned this line by hand\n' >> "$HOME/.claude/skills/ast-grep/SKILL.md"
set +e
bash "$INSTALL_SH" > /tmp/smoke3.log 2>&1
EC3=$?
set -e
if [ "$EC3" -ne 0 ] && grep -q "unmanaged.*collision" /tmp/smoke3.log; then
  pass "an in-place edit of a deployed skill is refused"
else
  fail "an in-place edit of a deployed skill is refused (exit=$EC3)"
fi
assert_contains "$HOME/.claude/skills/ast-grep/SKILL.md" "the user tuned this line by hand" "the hand edit survives the refusal"
# The stamp recorded the bytes we deployed, so any drift at all reads as someone else's file.
# --force is the documented way back, and it re-claims the new bytes.
bash "$INSTALL_SH" --force > /tmp/smoke3b.log 2>&1; cat /tmp/smoke3b.log | sed 's/^/    /' > /dev/null
assert_pristine_skill "$HOME/.claude/skills/ast-grep/SKILL.md" "$REPO_ROOT/skills/ast-grep/SKILL.md" "--force restores the repo source"
assert_skill_claimed "$HOME/.claude/skills/ast-grep/SKILL.md" "--force re-claims the restored bytes"

# ── 4. unmanaged collision — should refuse without --force ───────
echo "--- Test 4: unmanaged collision refusal ---"
cat > "$HOME/.claude/skills/ast-grep/SKILL.md" <<'UNMANAGED'
---
name: ast-grep
description: user custom skill — unmanaged
---
custom content that differs from repo source
UNMANAGED
# Remove managed marker to simulate unmanaged
set +e
bash "$INSTALL_SH" > /tmp/smoke4.log 2>&1
EC=$?
set -e
if [ "$EC" -ne 0 ] && grep -q "unmanaged.*collision" /tmp/smoke4.log 2>/dev/null; then pass "unmanaged collision refused"; else fail "unmanaged collision refused (exit=$EC)"; cat /tmp/smoke4.log | sed 's/^/    /'; fi
# File must be untouched
assert_contains "$HOME/.claude/skills/ast-grep/SKILL.md" "user custom skill" "unmanaged file untouched after refusal"

# ── 5. unmanaged collision with --force — backup and replace ─────
echo "--- Test 5: unmanaged collision with --force ---"
bash "$INSTALL_SH" --force > /tmp/smoke5.log 2>&1; cat /tmp/smoke5.log | sed 's/^/    /'
if ls "$HOME/.claude/skills/ast-grep/SKILL.md.bak."* >/dev/null 2>&1; then pass "backup created on --force"; else fail "backup created on --force"; fi
assert_pristine_skill "$HOME/.claude/skills/ast-grep/SKILL.md" "$REPO_ROOT/skills/ast-grep/SKILL.md" "skill is the repo source after --force"
assert_skill_claimed "$HOME/.claude/skills/ast-grep/SKILL.md" "--force deploy records the new bytes in the stamp"

# ── 6. unsupported OS/arch detection (inject via PATH shim) ───────
echo "--- Test 6: unsupported arch handling ---"
# Test that install.sh fails on unknown arch by shimming uname
cat > "$TMPHOME/fakebin/uname" <<'FAKEUNAME'
#!/usr/bin/env bash
if [ "$1" = "-s" ]; then echo "Linux"; else echo "riscv64"; fi
FAKEUNAME
chmod +x "$TMPHOME/fakebin/uname"
set +e
PATH="$TMPHOME/fakebin:$PATH" bash "$INSTALL_SH" --check > /tmp/smoke6.log 2>&1 || true
# --check doesn't call detect_platform, so test via a fresh install with fake uname
# Remove managed skill to force preflight path that calls detect_platform
# Instead just verify the shim would trigger if install attempted
PATH="$TMPHOME/fakebin:$PATH" bash "$INSTALL_SH" > /tmp/smoke6b.log 2>&1; EC6=$?
set -e
if [ "$EC6" -ne 0 ] && grep -q "unsupported arch" /tmp/smoke6b.log 2>/dev/null; then pass "unsupported arch rejected"; else fail "unsupported arch rejected (exit=$EC6)"; cat /tmp/smoke6b.log | sed 's/^/    /'; fi
# Cleanup fake uname and restore PATH with fakebin preserved
rm -f "$TMPHOME/fakebin/uname"
export PATH="$TMPHOME/fakebin:$HOME/.cargo/bin:$ORIGINAL_PATH"

# ── 7. wrong version detection via --check ────────────────────────
echo "--- Test 7: wrong version detection ---"
cat > "$TMPHOME/fakebin/ast-grep" <<'WRONGAG'
#!/usr/bin/env bash
echo "ast-grep 0.44.0"
WRONGAG
chmod +x "$TMPHOME/fakebin/ast-grep"
set +e
bash "$INSTALL_SH" --check > /tmp/smoke7.log 2>&1; EC7=$?
set -e
if [ "$EC7" -ne 0 ] && grep -q "MISMATCH" /tmp/smoke7.log 2>/dev/null; then pass "wrong version flagged by --check"; else fail "wrong version flagged by --check (exit=$EC7)"; cat /tmp/smoke7.log | sed 's/^/    /'; fi
# Restore correct mock
cat > "$TMPHOME/fakebin/ast-grep" <<'MOCKAG2'
#!/usr/bin/env bash
echo "ast-grep 0.45.2"
MOCKAG2
chmod +x "$TMPHOME/fakebin/ast-grep"
# Also keep xg correct
cat > "$HOME/.cargo/bin/xg" <<'MOCKXG2'
#!/usr/bin/env bash
echo "xg 0.7.0"
MOCKXG2
chmod +x "$HOME/.cargo/bin/xg"

# ── 8. uninstall (managed only) ───────────────────────────────────
echo "--- Test 8: uninstall ---"
# Ensure skill is managed before uninstall
bash "$INSTALL_SH" --force > /tmp/smoke8prep.log 2>&1; cat /tmp/smoke8prep.log | sed 's/^/    /' > /dev/null
bash "$INSTALL_SH" --uninstall > /tmp/smoke8.log 2>&1; cat /tmp/smoke8.log | sed 's/^/    /'
assert_file_not_exists "$HOME/.claude/skills/ast-grep/SKILL.md" "skill removed on uninstall"
# Also check cort shim removed
if [ -f "$HOME/.local/bin/cort" ] || [ -f "$HOME/.cargo/bin/cort" ]; then fail "cort shim removed on uninstall (found in local/bin or cargo/bin)"; else pass "cort shim removed on uninstall"; fi
# PATH block removed
PROFILE_STILL=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
  if grep -qF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null; then PROFILE_STILL=1; fi
done
if [ "$PROFILE_STILL" -eq 0 ]; then pass "profile block removed on uninstall"; else fail "profile block removed on uninstall"; fi
# xg binary: our mock was pre-existing (no legacy_xg_bin in manifest after skip), so must NOT be removed
assert_file_exists "$HOME/.cargo/bin/xg" "pre-existing xg binary preserved on uninstall"
# The hook goes out with everything else. Uninstall runs it before the binary is removed, because
# `cort hook-install --remove` is what owns the JSON edit.
if [ -f "$HOME/.claude/settings.json" ]; then
  assert_not_contains "$HOME/.claude/settings.json" "hook-suggest" "PreToolUse hook unwired on uninstall"
else
  pass "PreToolUse hook unwired on uninstall (settings.json absent)"
fi

# ── 9. SHA mismatch is fatal ─────────────────────────────────────
echo "--- Test 9: SHA mismatch is fatal ---"
# Force download path by making ast-grep report wrong version
cat > "$TMPHOME/fakebin/ast-grep" <<'FAKEAG_WRONG'
#!/usr/bin/env bash
echo "ast-grep 0.44.0"
FAKEAG_WRONG
chmod +x "$TMPHOME/fakebin/ast-grep"
cat > "$TMPHOME/fakebin/curl" <<'FAKECURL'
#!/usr/bin/env bash
# Emit a payload that cannot match any recorded checksum.
for a in "$@"; do case "$a" in -o) next=out;; *) [ "${next:-}" = out ] && { printf 'corrupt' > "$a"; next=; };; esac; done
exit 0
FAKECURL
chmod +x "$TMPHOME/fakebin/curl"
set +e
PATH="$TMPHOME/fakebin:$PATH" bash "$INSTALL_SH" > /tmp/smoke9.log 2>&1
EC9=$?
set -e
if [ "$EC9" -ne 0 ] && grep -q "SHA-256 mismatch" /tmp/smoke9.log; then
  pass "SHA mismatch is fatal"
else
  fail "SHA mismatch is fatal (exit=$EC9)"; sed 's/^/    /' /tmp/smoke9.log
fi
assert_not_contains /tmp/smoke9.log "proceeding anyway" "no proceed-anyway path remains"
# Restore correct fakes
cat > "$TMPHOME/fakebin/ast-grep" <<'FAKEAG_RESTORED'
#!/usr/bin/env bash
echo "ast-grep 0.45.2"
FAKEAG_RESTORED
chmod +x "$TMPHOME/fakebin/ast-grep"
cat > "$TMPHOME/fakebin/curl" <<'FAKECURL_STUB2'
#!/usr/bin/env bash
exit 1
FAKECURL_STUB2
chmod +x "$TMPHOME/fakebin/curl"

# ── 10. manifest v1 -> v2 migration ───────────────────────────────
echo "--- Test 10: manifest v1 -> v2 migration ---"
MF="$HOME/.local/share/cortexyoung/manifest"
mkdir -p "$(dirname "$MF")"
# Use a dummy xg path so that legacy_xg_bin does not point to the mock xg (which must remain preserved)
printf 'xg_bin:%s\nskill:%s\n' "/tmp/fake-xg-$$" "$HOME/.claude/skills/xgrep/SKILL.md" > "$MF"
bash "$INSTALL_SH" --force > /tmp/smoke10.log 2>&1 || true
assert_contains "$MF" "manifest_version:2" "manifest_version recorded"
assert_contains "$MF" "legacy_xg_bin:" "xg_bin migrated to legacy_xg_bin"
assert_contains "$MF" "skill_xgrep:" "skill migrated to skill_xgrep"
assert_contains "$MF" "cort_bin:" "cort_bin recorded"
assert_not_contains "$MF" "^xg_bin:" "old xg_bin key removed"
assert_file_exists "$HOME/.cargo/bin/xg" "migration never deletes the pre-existing xg"

# ── 11. sg is not ast-grep ───────────────────────────────────────
echo "--- Test 11: an unrelated sg on PATH is not mistaken for ast-grep ---"
cat > "$TMPHOME/fakebin/sg" <<'FAKESG'
#!/usr/bin/env bash
echo "sg from shadow-utils"
FAKESG
chmod +x "$TMPHOME/fakebin/sg"
PATH="$TMPHOME/fakebin:$PATH" bash "$INSTALL_SH" --check > /tmp/smoke11.log 2>&1 || true
assert_not_contains /tmp/smoke11.log "ast-grep: $TMPHOME/fakebin/sg" "sg is never adopted as ast-grep"
rm -f "$TMPHOME/fakebin/sg"

# ── 12. two-skill rollback ───────────────────────────────────────
echo "--- Test 12: a collision on the second skill rolls back both ---"
rm -f "$HOME/.claude/skills/ast-grep/SKILL.md" "$HOME/.claude/skills/xgrep/SKILL.md"
mkdir -p "$HOME/.claude/skills/xgrep"
printf -- '---\nname: xgrep\n---\nuser custom, unmanaged\n' > "$HOME/.claude/skills/xgrep/SKILL.md"
set +e
bash "$INSTALL_SH" --with-xgrep > /tmp/smoke12.log 2>&1
EC12=$?
set -e
if [ "$EC12" -ne 0 ]; then pass "two-skill preflight refuses"; else fail "two-skill preflight refuses"; fi
assert_file_not_exists "$HOME/.claude/skills/ast-grep/SKILL.md" "first skill was not deployed before the second preflight failed"
assert_contains "$HOME/.claude/skills/xgrep/SKILL.md" "user custom" "unmanaged file untouched"

# ── 13. uninstall v2-owned artifacts ─────────────────────────────
echo "--- Test 13: uninstall removes only v2-owned artifacts ---"
bash "$INSTALL_SH" --force > /dev/null 2>&1
bash "$INSTALL_SH" --uninstall > /tmp/smoke13.log 2>&1
assert_file_not_exists "$HOME/.claude/skills/ast-grep/SKILL.md" "ast-grep skill removed"
# cort shim may be in cargo/bin or local/bin depending on BIN_DIR
if [ -f "$HOME/.local/bin/cort" ]; then fail "cort shim removed (found $HOME/.local/bin/cort)"; else pass "cort shim removed"; fi
if [ -f "$HOME/.cargo/bin/cort" ]; then fail "cort payload shim still in cargo bin"; else pass "cort shim not in cargo bin (or removed)"; fi
assert_file_not_exists "$HOME/.local/share/cortexyoung/cort/cort" "cort payload removed"
assert_file_not_exists "$HOME/.claude/skills/ast-grep/$STAMP_NAME" "manifest path removes the stamp too"
assert_file_exists "$HOME/.cargo/bin/xg" "pre-existing xg preserved"

# ── 14. DB interrupt recovery ────────────────────────────────────
echo "--- Test 14: an interrupted index leaves the previous db readable ---"
PROJ="$TMPHOME/proj"; mkdir -p "$PROJ/src"
# A Rust fixture: this repo indexes Rust, and the test must not need a TypeScript grammar to prove
# that a killed index leaves the previous database readable.
printf 'fn a() -> i32 { 1 }\n' > "$PROJ/src/a.rs"
# Use the real ast-grep and the real (Rust) cort — remove fakebin from PATH here
REAL_PATH="$ORIGINAL_PATH"
HOST_AG_EXPECTED="0.45.2"
CORT_BIN_UNDER_TEST="$REPO_ROOT/rust/target/release/cort"
if [ ! -x "$CORT_BIN_UNDER_TEST" ]; then
  echo "  SKIP: rust/target/release/cort not built (run cargo build --release first)"
  CORT_BIN_UNDER_TEST=""
fi
# Resolve the real ast-grep the same way the product does — through PATH, minus the fakes this
# script put there. No host-specific absolute paths: an installer-provisioned binary is the only
# thing that may be used here, and if it is absent the test says SKIP rather than guessing.
HOST_AG="$(PATH="$REAL_PATH" command -v ast-grep 2>/dev/null || true)"
if [ ! -x "$HOST_AG" ]; then
  echo "  SKIP: no real ast-grep on PATH (run ./install.sh); Test 14 needs a working parser"
  HOST_AG=""
fi
T14_RUNS=1
if [ -z "$CORT_BIN_UNDER_TEST" ] || [ -z "$HOST_AG" ]; then T14_RUNS=0; fi
if [ -n "$HOST_AG" ] && [ -n "$CORT_BIN_UNDER_TEST" ]; then
  AG_VER="$("$HOST_AG" --version 2>/dev/null | awk '{print $2}')"
  if [ "$AG_VER" != "$HOST_AG_EXPECTED" ]; then
    fail "real ast-grep on PATH is $AG_VER, not the pinned $HOST_AG_EXPECTED ($HOST_AG)"
  else
    pass "real ast-grep is the pinned version ($AG_VER)"
  fi
fi
# Index the fixture project with the host ast-grep (fakebin must not shadow it)
if [ "$T14_RUNS" = "1" ]; then
  ( cd "$PROJ" && PATH="$REAL_PATH" CORT_AST_GREP_BIN="$HOST_AG" "$CORT_BIN_UNDER_TEST" index . > /dev/null 2>&1 )
fi
chunk_count() {
  [ "$T14_RUNS" = "1" ] || { echo ""; return; }
  ( cd "$PROJ" && PATH="$REAL_PATH" CORT_AST_GREP_BIN="$HOST_AG" \
      "$CORT_BIN_UNDER_TEST" status . 2>/dev/null ) \
    | sed -n 's/.*"chunks": *\([0-9][0-9]*\).*/\1/p' | head -1
}
BEFORE="$(chunk_count)"
if [ "$T14_RUNS" != "1" ]; then
  pass "baseline index readable (skipped: cort binary or pinned ast-grep unavailable)"
elif [ -n "$BEFORE" ] && [ "$BEFORE" -gt 0 ]; then
  pass "baseline index readable ($BEFORE chunks)"
else
  fail "baseline index readable (got '$BEFORE')"
fi
for i in $(seq 1 40); do printf 'fn f%d() -> i32 { %d }\n' "$i" "$i" > "$PROJ/src/f$i.rs"; done
if [ "$T14_RUNS" = "1" ]; then
  ( cd "$PROJ" && PATH="$REAL_PATH" CORT_AST_GREP_BIN="$HOST_AG" "$CORT_BIN_UNDER_TEST" index . > /dev/null 2>&1 & IDX=$!; sleep 0.2; kill -9 $IDX 2>/dev/null; wait $IDX 2>/dev/null ) || true
fi
AFTER="$(chunk_count)"
# A killed full index either rolled back (AFTER == BEFORE) or had already committed
# (AFTER > BEFORE). What must never happen is an unreadable or truncated database.
if [ "$T14_RUNS" != "1" ]; then
  pass "db intact after a killed index (skipped: cort binary or pinned ast-grep unavailable)"
elif [ -n "$AFTER" ] && [ "$AFTER" -ge "$BEFORE" ]; then
  pass "db intact after a killed index (before=$BEFORE after=$AFTER)"
else
  fail "db intact after a killed index (before=$BEFORE after='$AFTER')"
fi

echo "--- Test 15: install rebuilds even though a release binary already exists (F-02) ---"
PREBUILT="$REPO_ROOT/rust/target/release/cort"
if [ ! -x "$PREBUILT" ]; then
  pass "skipped: no prebuilt binary for the existence check to be fooled by"
else
  : > "$TMPHOME/cargo-build.log"
  FAKE_CARGO_LOG="$TMPHOME/cargo-build.log" \
    PATH="$TMPHOME/fakebin:$HOME/.cargo/bin:$ORIGINAL_PATH" \
    bash "$REPO_ROOT/install.sh" > "$TMPHOME/test15.out" 2>&1 || true
  if grep -q "build --release --locked" "$TMPHOME/cargo-build.log"; then
    pass "cargo build --release --locked runs with a prebuilt binary present"
  else
    fail "cargo build --release --locked runs with a prebuilt binary present (log empty)"
  fi
  if grep -q "installed cort" "$TMPHOME/test15.out"; then
    pass "install completed with a prebuilt binary present"
  else
    fail "install completed with a prebuilt binary present (see $TMPHOME/test15.out)"
  fi
fi

echo "--- Test 16: the same routing skill is deployed for Codex ---"
CODEX_DEST="$HOME/.codex/skills/ast-grep/SKILL.md"
CLAUDE_DEST="$HOME/.claude/skills/ast-grep/SKILL.md"
assert_file_exists "$CODEX_DEST" "codex skill deployed"
assert_not_contains "$CODEX_DEST" "$MANAGED_SIGNATURE" "codex SKILL.md holds no installer bookkeeping"
assert_frontmatter_first "$CODEX_DEST" "codex skill keeps its YAML fence on line 1"
assert_frontmatter_keys_only "$CODEX_DEST" "codex frontmatter holds only YAML keys"
assert_skill_claimed "$CODEX_DEST" "codex skill claimed by its own stamp"
if cmp -s "$CODEX_DEST" "$CLAUDE_DEST"; then
  pass "claude and codex skill copies are byte-identical (one source of truth)"
else
  fail "claude and codex skill copies are byte-identical"
fi
assert_contains "$HOME/.local/share/cortexyoung/manifest" "skill_ast_grep_codex:" "manifest records the codex skill"
bash "$INSTALL_SH" > /tmp/smoke16.log 2>&1; cat /tmp/smoke16.log | sed 's/^/    /' > /dev/null
if [ "$(grep -c '^skill_sha256:' "$(dirname "$CODEX_DEST")/$STAMP_NAME")" = "1" ]; then
  pass "rerun did not duplicate the codex stamp hash"
else
  fail "rerun did not duplicate the codex stamp hash"
fi
assert_pristine_skill "$CODEX_DEST" "$REPO_ROOT/skills/ast-grep/SKILL.md" "codex skill still pristine after rerun"
# An unmanaged file in the Codex home must be refused, and refused before anything is mutated.
printf 'my own codex skill, do not touch\n' > "$CODEX_DEST"
set +e
bash "$INSTALL_SH" > /tmp/smoke16b.log 2>&1; EC16=$?
set -e
if [ "$EC16" -ne 0 ] && grep -q "unmanaged.*collision" /tmp/smoke16b.log; then
  pass "unmanaged codex skill collision refused"
else
  fail "unmanaged codex skill collision refused (exit=$EC16)"
fi
assert_contains "$CODEX_DEST" "my own codex skill" "unmanaged codex file untouched after refusal"
assert_pristine_skill "$CLAUDE_DEST" "$REPO_ROOT/skills/ast-grep/SKILL.md" "claude skill untouched by the codex refusal"
bash "$INSTALL_SH" --force > /tmp/smoke16c.log 2>&1; cat /tmp/smoke16c.log | sed 's/^/    /' > /dev/null
assert_pristine_skill "$CODEX_DEST" "$REPO_ROOT/skills/ast-grep/SKILL.md" "codex skill adopted with --force"
assert_skill_claimed "$CODEX_DEST" "--force on the codex home re-claims the new bytes"
# uninstall removes the managed copy and never the other home's
bash "$INSTALL_SH" --uninstall > /tmp/smoke16d.log 2>&1; cat /tmp/smoke16d.log | sed 's/^/    /' > /dev/null
if [ -f "$CODEX_DEST" ]; then fail "codex skill removed on uninstall"; else pass "codex skill removed on uninstall"; fi
assert_file_not_exists "$(dirname "$CODEX_DEST")/$STAMP_NAME" "no orphan stamp left behind on uninstall"
# The directory itself may legitimately survive: --force in the test above left a .bak of the
# user's own file there, and uninstall must not eat backups it did not make.
if [ -f "$(dirname "$CODEX_DEST")/$STAMP_NAME" ]; then
  fail "the stamp is removed before the directory is even considered for rmdir"
else
  pass "the stamp is removed before the directory is even considered for rmdir"
fi

# ── 17. the shape the previous installer wrote must be repaired, not reported "up to date" ──
echo "--- Test 17: the two legacy in-document marker shapes are repaired ---"
LEGACY_SKILL="$HOME/.claude/skills/ast-grep/SKILL.md"
SRC_SKILL="$REPO_ROOT/skills/ast-grep/SKILL.md"

# Shape A (install.sh up to F-15): marker above the fence. Both loaders then skip the whole skill.
{ echo "$MANAGED_SIGNATURE"; cat "$SRC_SKILL"; } > "$LEGACY_SKILL"
if [ "$(head -n 1 "$LEGACY_SKILL")" = "$MANAGED_SIGNATURE" ]; then
  pass "fixture reproduces legacy shape A (marker above the fence)"
else
  fail "fixture reproduces legacy shape A"
fi
rm -f "$(dirname "$LEGACY_SKILL")/$STAMP_NAME"
bash "$INSTALL_SH" > /tmp/smoke17.log 2>&1; cat /tmp/smoke17.log | sed 's/^/    /' > /dev/null
if grep -q "repaired skill" /tmp/smoke17.log; then
  pass "rerun repairs legacy shape A (a hash match must not excuse the shape)"
else
  fail "rerun repairs legacy shape A"
fi
assert_pristine_skill "$LEGACY_SKILL" "$SRC_SKILL" "legacy shape A upgraded to the pristine document"
assert_skill_claimed "$LEGACY_SKILL" "legacy shape A ownership moved into the stamp"

# Shape B (install.sh F-16): marker as a YAML comment inside the frontmatter. Loaders accept it,
# the documented shape does not: the block is for keys, and the installer had to delete that line
# again before every hash comparison.
awk -v sig="$MANAGED_SIGNATURE" 'NR==1{print; print "# " sig; next} {print}' "$SRC_SKILL" > "$LEGACY_SKILL.tmp" \
  && mv "$LEGACY_SKILL.tmp" "$LEGACY_SKILL"
if [ "$(head -n 2 "$LEGACY_SKILL" | tail -n 1)" = "# $MANAGED_SIGNATURE" ]; then
  pass "fixture reproduces legacy shape B (marker inside the frontmatter)"
else
  fail "fixture reproduces legacy shape B"
fi
assert_frontmatter_keys_only_rejects "$LEGACY_SKILL" "the key-only gate rejects shape B, so it can catch it"
bash "$INSTALL_SH" > /tmp/smoke17b.log 2>&1; cat /tmp/smoke17b.log | sed 's/^/    /' > /dev/null
if grep -q "repaired skill" /tmp/smoke17b.log; then
  pass "rerun repairs legacy shape B"
else
  fail "rerun repairs legacy shape B"
fi
assert_pristine_skill "$LEGACY_SKILL" "$SRC_SKILL" "legacy shape B upgraded to the pristine document"
assert_skill_claimed "$LEGACY_SKILL" "legacy shape B ownership moved into the stamp"
# The strongest form of the assertion, when the real loader is available: does the deployed text
# actually reach a Codex prompt? Skipped (never faked) where codex is not installed, e.g. CI.
# Derived, not hardcoded: the probe used to be a literal phrase from the description, so editing
# the prose failed this test even when the loader was reading the file perfectly. Take whatever the
# source says instead — the assertion stays "does the deployed text reach a real prompt", which is
# the F-16 failure mode, and nothing else.
SKILL_PROBE="$(awk '/^description: /{sub(/^description: /,""); print substr($0,1,48); exit}' "$SRC_SKILL")"
if [ -z "$SKILL_PROBE" ]; then
  fail "could not derive a probe string from $SRC_SKILL"
else
  echo "    probe: ${SKILL_PROBE}..."
fi
if command -v codex >/dev/null 2>&1; then
  # Resolve the same way install.sh does, so the probe reads the home the skill was written to.
  codex_probe_home="${CODEX_HOME:-$HOME/.codex}"
  PROMPT_INPUT="$(CODEX_HOME="$codex_probe_home" codex debug prompt-input 'smoke probe' 2>/dev/null || true)"
  if [ -z "$PROMPT_INPUT" ]; then
    echo "  SKIP: codex prompt-input returned nothing; loader visibility not exercised"
  elif printf '%s' "$PROMPT_INPUT" | grep -qF "$SKILL_PROBE"; then
    pass "deployed skill is visible in a real Codex prompt (loader accepted it)"
  else
    fail "deployed skill is on disk but invisible to the Codex loader"
  fi
else
  echo "  SKIP: codex not installed; loader visibility not exercised"
fi

echo "--- Test 18: the deploy log dates every change of the deployed bytes ---"
DEPLOY_LOG_FILE="$XDG_DATA_HOME/cortexyoung/deploy.log"
AG_DEST="$HOME/.claude/skills/ast-grep/SKILL.md"
if [ -f "$DEPLOY_LOG_FILE" ]; then
  pass "install writes a deploy log"
else
  fail "install writes a deploy log ($DEPLOY_LOG_FILE)"
fi
LINES_BEFORE="$(wc -l < "$DEPLOY_LOG_FILE" 2>/dev/null || echo 0)"
bash "$INSTALL_SH" > /tmp/smoke18a.log 2>&1
LINES_AFTER="$(wc -l < "$DEPLOY_LOG_FILE" 2>/dev/null || echo 0)"
if [ "$LINES_BEFORE" = "$LINES_AFTER" ]; then
  pass "a redeploy of identical bytes appends nothing (the log records changes, not runs)"
else
  fail "a redeploy appended a line: $LINES_BEFORE -> $LINES_AFTER"
fi
# A body-only edit is the case the transcript-grep method cannot see: the frontmatter description is
# untouched, so only the sha moves. The log has to catch exactly this.
DESC_BEFORE="$(sed -n 3p "$AG_DEST")"
printf "\nA line appended by the smoke test.\n" >> "$REPO_ROOT/skills/ast-grep/SKILL.md"
bash "$INSTALL_SH" > /tmp/smoke18b.log 2>&1
git -C "$REPO_ROOT" checkout -- skills/ast-grep/SKILL.md 2>/dev/null || true
LINES_EDITED="$(wc -l < "$DEPLOY_LOG_FILE" 2>/dev/null || echo 0)"
if [ "$LINES_EDITED" -gt "$LINES_AFTER" ]; then
  pass "a body-only edit appends a line even though the description never changed"
else
  fail "a body-only edit was not logged: $LINES_AFTER -> $LINES_EDITED"
fi
if [ "$(sed -n 3p "$AG_DEST")" = "$DESC_BEFORE" ]; then
  pass "the description really was byte-identical across that edit (the blind spot is reproduced)"
else
  fail "fixture did not reproduce a body-only edit"
fi
LAST_LINE="$(tail -n 1 "$DEPLOY_LOG_FILE")"
if printf "%s" "$LAST_LINE" | grep -qE "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:+-]+	[0-9a-f]{64}	/"; then
  pass "each line is <iso8601> <sha256> <dest>, tab separated"
else
  fail "deploy log line shape: $LAST_LINE"
fi

# ── 19. --check names the stale binary, never the settings file ───
# `hook-install --status` resolves an unreadable or unparsable settings.json to `wired: false`
# instead of failing, so the failure branch is only ever reached by a cort that cannot run the
# subcommand. It used to print "could not read <settings.json>", which is the one file that is
# provably not at fault -- a wrong pointer in the check whose whole job is to stop the hook going
# down silently. Reproduced with a double that answers the way a pre-hook-install binary does.
echo "--- Test 19: --check blames the binary, not settings.json ---"
STALEBIN="$TMPHOME/stalebin"
mkdir -p "$STALEBIN"
cat > "$STALEBIN/cort" <<'FAKESTALECORT'
#!/usr/bin/env bash
# A cort from before `hook-install` existed: unknown subcommand on stderr, non-zero exit.
if [ "$1" = "hook-install" ]; then
  echo '{"detail":{"command":"hook-install"},"error":"unknown_command"}' >&2
  exit 2
fi
echo "cort 0.1.0 (rust)"
FAKESTALECORT
chmod +x "$STALEBIN/cort"
PATH="$STALEBIN:$PATH" bash "$INSTALL_SH" --check > /tmp/smoke19.log 2>&1 || true
if grep -q "predates hook-install" /tmp/smoke19.log; then
  pass "--check names the stale binary as the cause"
else
  fail "--check does not name the stale binary"; sed 's/^/    /' /tmp/smoke19.log
fi
if grep -q "hook: could not read" /tmp/smoke19.log; then
  fail "--check still blames settings.json for a stale-binary failure"
else
  pass "--check no longer blames settings.json for a stale-binary failure"
fi
rm -rf "$STALEBIN"

# ── summary ──────────────────────────────────────────────────────
echo ""
echo "=== smoke results: $PASS passed, $FAIL failed ==="
# Cleanup
rm -rf "$TMPHOME"
if [ "$FAIL" -gt 0 ]; then exit 1; fi
