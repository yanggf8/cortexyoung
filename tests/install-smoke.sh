#!/usr/bin/env bash
set -euo pipefail
# Offline smoke test for install.sh — mocked download/cargo, temp HOME
# Covers: first-install, identical rerun, managed update, unmanaged-collision
#         refusal, unsupported OS/arch, wrong version detection, profile
#         idempotency, uninstall.
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

# Mock xg binary (so --check / version logic works without download)
cat > "$HOME/.cargo/bin/xg" <<'MOCKXG'
#!/usr/bin/env bash
echo "xg 0.7.0"
MOCKXG
chmod +x "$HOME/.cargo/bin/xg"
export PATH="$HOME/.cargo/bin:$PATH"

# Mock curl/wget to avoid network: intercept install.sh download path by
# pre-creating a fake asset? Instead we test the skill/profile/manifest
# paths directly — binary install is skipped because mock xg already matches.

echo "=== install-smoke: temp HOME=$TMPHOME ==="

# ── 1. first install ─────────────────────────────────────────────
echo "--- Test 1: first install ---"
bash "$INSTALL_SH" 2>&1 | sed 's/^/    /'
assert_file_exists "$HOME/.claude/skills/xgrep/SKILL.md" "skill installed"
assert_contains "$HOME/.claude/skills/xgrep/SKILL.md" "$MANAGED_MARKER" "skill has managed marker"
assert_file_exists "$HOME/.local/share/cortexyoung/manifest" "manifest created"
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
MANIFEST_BEFORE="$(cat "$HOME/.local/share/cortexyoung/manifest" 2>/dev/null || true)"
# Count marker occurrences before
MARKER_COUNT_BEFORE=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
  if [ -f "$p" ]; then
    c="$(grep -cF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null || true)"
    MARKER_COUNT_BEFORE=$((MARKER_COUNT_BEFORE + c))
  fi
done
bash "$INSTALL_SH" 2>&1 | sed 's/^/    /'
MARKER_COUNT_AFTER=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
  if [ -f "$p" ]; then
    c="$(grep -cF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null || true)"
    MARKER_COUNT_AFTER=$((MARKER_COUNT_AFTER + c))
  fi
done
if [ "$MARKER_COUNT_BEFORE" -eq "$MARKER_COUNT_AFTER" ]; then pass "profile block idempotent (no duplication)"; else fail "profile block idempotent ($MARKER_COUNT_BEFORE -> $MARKER_COUNT_AFTER)"; fi
assert_contains "$HOME/.claude/skills/xgrep/SKILL.md" "$MANAGED_MARKER" "skill still managed after rerun"

# ── 3. managed update (installer replaces outdated managed skill) ─
echo "--- Test 3: managed update ---"
echo "$MANAGED_MARKER" > "$HOME/.claude/skills/xgrep/SKILL.md"
echo "stale content" >> "$HOME/.claude/skills/xgrep/SKILL.md"
bash "$INSTALL_SH" 2>&1 | sed 's/^/    /'
if grep -qF "stale content" "$HOME/.claude/skills/xgrep/SKILL.md" 2>/dev/null; then fail "managed update replaced stale content"; else pass "managed update replaced stale content"; fi

# ── 4. unmanaged collision — should refuse without --force ───────
echo "--- Test 4: unmanaged collision refusal ---"
cat > "$HOME/.claude/skills/xgrep/SKILL.md" <<'UNMANAGED'
---
name: xgrep
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
assert_contains "$HOME/.claude/skills/xgrep/SKILL.md" "user custom skill" "unmanaged file untouched after refusal"

# ── 5. unmanaged collision with --force — backup and replace ─────
echo "--- Test 5: unmanaged collision with --force ---"
bash "$INSTALL_SH" --force 2>&1 | sed 's/^/    /'
if ls "$HOME/.claude/skills/xgrep/SKILL.md.bak."* >/dev/null 2>&1; then pass "backup created on --force"; else fail "backup created on --force"; fi
assert_contains "$HOME/.claude/skills/xgrep/SKILL.md" "$MANAGED_MARKER" "skill managed after --force"

# ── 6. unsupported OS/arch detection (inject via PATH shim) ───────
echo "--- Test 6: unsupported arch handling ---"
# Test that install.sh fails on unknown arch by shimming uname
mkdir -p "$TMPHOME/fakebin"
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
# Restore PATH
export PATH="$HOME/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# ── 7. wrong version detection via --check ────────────────────────
echo "--- Test 7: wrong version detection ---"
cat > "$HOME/.cargo/bin/xg" <<'WRONGVER'
#!/usr/bin/env bash
echo "xg 0.6.0"
WRONGVER
chmod +x "$HOME/.cargo/bin/xg"
set +e
bash "$INSTALL_SH" --check > /tmp/smoke7.log 2>&1; EC7=$?
set -e
if [ "$EC7" -ne 0 ] && grep -q "MISMATCH" /tmp/smoke7.log 2>/dev/null; then pass "wrong version flagged by --check"; else fail "wrong version flagged by --check (exit=$EC7)"; cat /tmp/smoke7.log | sed 's/^/    /'; fi
# Restore correct mock
cat > "$HOME/.cargo/bin/xg" <<'MOCKXG2'
#!/usr/bin/env bash
echo "xg 0.7.0"
MOCKXG2
chmod +x "$HOME/.cargo/bin/xg"

# ── 8. uninstall (managed only) ───────────────────────────────────
echo "--- Test 8: uninstall ---"
# Ensure skill is managed before uninstall
bash "$INSTALL_SH" --force 2>&1 | sed 's/^/    /' > /dev/null
bash "$INSTALL_SH" --uninstall 2>&1 | sed 's/^/    /'
assert_file_not_exists "$HOME/.claude/skills/xgrep/SKILL.md" "skill removed on uninstall"
# PATH block removed
PROFILE_STILL=0
for p in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
  if grep -qF "# >>> cortexyoung xg >>>" "$p" 2>/dev/null; then PROFILE_STILL=1; fi
done
if [ "$PROFILE_STILL" -eq 0 ]; then pass "profile block removed on uninstall"; else fail "profile block removed on uninstall"; fi
# xg binary: our mock was pre-existing (no xg_bin in manifest after skip), so must NOT be removed
assert_file_exists "$HOME/.cargo/bin/xg" "pre-existing xg binary preserved on uninstall"

# ── summary ──────────────────────────────────────────────────────
echo ""
echo "=== smoke results: $PASS passed, $FAIL failed ==="
# Cleanup
rm -rf "$TMPHOME"
if [ "$FAIL" -gt 0 ]; then exit 1; fi
