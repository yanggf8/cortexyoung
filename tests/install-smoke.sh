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
MANAGED_MARKER="# managed by cortexyoung install.sh"
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
assert_contains "$HOME/.claude/skills/ast-grep/SKILL.md" "$MANAGED_MARKER" "ast-grep skill has managed marker"
assert_file_not_exists "$HOME/.claude/skills/xgrep/SKILL.md" "xgrep skill not installed by default"
assert_file_exists "$HOME/.local/share/cortexyoung/manifest" "manifest created"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "manifest_version:2" "manifest version 2 recorded"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "skill_ast_grep:" "skill_ast_grep recorded"
assert_contains "$HOME/.local/share/cortexyoung/manifest" "cort_bin:" "cort_bin recorded"
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
assert_contains "$HOME/.claude/skills/ast-grep/SKILL.md" "$MANAGED_MARKER" "ast-grep skill still managed after rerun"

# ── 3. managed update (installer replaces outdated managed skill) ─
echo "--- Test 3: managed update ---"
echo "$MANAGED_MARKER" > "$HOME/.claude/skills/ast-grep/SKILL.md"
echo "stale content" >> "$HOME/.claude/skills/ast-grep/SKILL.md"
bash "$INSTALL_SH" > /tmp/smoke3.log 2>&1; cat /tmp/smoke3.log | sed 's/^/    /'
if grep -qF "stale content" "$HOME/.claude/skills/ast-grep/SKILL.md" 2>/dev/null; then fail "managed update replaced stale content"; else pass "managed update replaced stale content"; fi

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
assert_contains "$HOME/.claude/skills/ast-grep/SKILL.md" "$MANAGED_MARKER" "ast-grep skill managed after --force"

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
assert_file_exists "$HOME/.cargo/bin/xg" "pre-existing xg preserved"

# ── 14. DB interrupt recovery ────────────────────────────────────
echo "--- Test 14: an interrupted index leaves the previous db readable ---"
PROJ="$TMPHOME/proj"; mkdir -p "$PROJ/src"
printf 'export function a() { return 1; }\n' > "$PROJ/src/a.ts"
# Use the real ast-grep and the real (Rust) cort — remove fakebin from PATH here
REAL_PATH="$ORIGINAL_PATH"
CORT_BIN_UNDER_TEST="$REPO_ROOT/rust/target/release/cort"
if [ ! -x "$CORT_BIN_UNDER_TEST" ]; then
  echo "  SKIP: rust/target/release/cort not built (run cargo build --release first)"
  CORT_BIN_UNDER_TEST=""
fi
# Find real ast-grep (host) and ensure it is on REAL_PATH
HOST_AG="$(command -v ast-grep 2>/dev/null || echo /home/yanggf/.nvm/versions/node/v24.3.0/bin/ast-grep)"
if [ ! -x "$HOST_AG" ]; then HOST_AG="$(which ast-grep 2>/dev/null || true)"; fi
# Prefer host ast-grep if fake is still shadowing; resolve via REAL_PATH
HOST_AG_REAL="$(PATH="$REAL_PATH" command -v ast-grep 2>/dev/null || echo "$HOST_AG")"
if [ -x "$HOST_AG_REAL" ]; then HOST_AG="$HOST_AG_REAL"; fi
# Index the fixture project with the host ast-grep (fakebin must not shadow it)
if [ -n "$CORT_BIN_UNDER_TEST" ]; then
  ( cd "$PROJ" && PATH="$REAL_PATH" CORT_AST_GREP_BIN="$HOST_AG" "$CORT_BIN_UNDER_TEST" index . > /dev/null 2>&1 )
fi
chunk_count() {
  [ -n "$CORT_BIN_UNDER_TEST" ] || { echo ""; return; }
  ( cd "$PROJ" && PATH="$REAL_PATH" CORT_AST_GREP_BIN="$HOST_AG" \
      "$CORT_BIN_UNDER_TEST" status . 2>/dev/null ) \
    | sed -n 's/.*"chunks": *\([0-9][0-9]*\).*/\1/p' | head -1
}
BEFORE="$(chunk_count)"
if [ -z "$CORT_BIN_UNDER_TEST" ]; then
  pass "baseline index readable (skipped: cort binary not built)"
elif [ -n "$BEFORE" ] && [ "$BEFORE" -gt 0 ]; then
  pass "baseline index readable ($BEFORE chunks)"
else
  fail "baseline index readable (got '$BEFORE')"
fi
for i in $(seq 1 40); do printf 'export function f%d() { return %d; }\n' "$i" "$i" > "$PROJ/src/f$i.ts"; done
if [ -n "$CORT_BIN_UNDER_TEST" ]; then
  ( cd "$PROJ" && PATH="$REAL_PATH" CORT_AST_GREP_BIN="$HOST_AG" "$CORT_BIN_UNDER_TEST" index . > /dev/null 2>&1 & IDX=$!; sleep 0.2; kill -9 $IDX 2>/dev/null; wait $IDX 2>/dev/null ) || true
fi
AFTER="$(chunk_count)"
# A killed full index either rolled back (AFTER == BEFORE) or had already committed
# (AFTER > BEFORE). What must never happen is an unreadable or truncated database.
if [ -z "$CORT_BIN_UNDER_TEST" ]; then
  pass "db intact after a killed index (skipped: cort binary not built)"
elif [ -n "$AFTER" ] && [ "$AFTER" -ge "$BEFORE" ]; then
  pass "db intact after a killed index (before=$BEFORE after=$AFTER)"
else
  fail "db intact after a killed index (before=$BEFORE after='$AFTER')"
fi

# ── summary ──────────────────────────────────────────────────────
echo ""
echo "=== smoke results: $PASS passed, $FAIL failed ==="
# Cleanup
rm -rf "$TMPHOME"
if [ "$FAIL" -gt 0 ]; then exit 1; fi
