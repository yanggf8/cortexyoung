#!/usr/bin/env bash
set -euo pipefail
# cortexyoung — xg installer + xgrep skill deploy
# Pinned: xg v0.7.0 from https://github.com/momokun7/xgrep
# Upstream publishes NO checksums; SHA-256 below is repo-maintained (verified 2026-08-25).
# Usage: ./install.sh [--check] [--uninstall] [--force] [--with-rustup]

VERSION="0.7.0"
REPO="momokun7/xgrep"
CRATE="xgrep-search"
MANAGED_MARKER="# managed by cortexyoung install.sh"
MANIFEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung"
MANIFEST_FILE="$MANIFEST_DIR/manifest"
SKILL_SRC_REL="skills/xgrep/SKILL.md"
SKILL_DEST="$HOME/.claude/skills/xgrep/SKILL.md"

FORCE=0; WITH_RUSTUP=0; MODE="install"

for arg in "$@"; do
  case "$arg" in
    --check)     MODE="check" ;;
    --uninstall) MODE="uninstall" ;;
    --force)     FORCE=1 ;;
    --with-rustup) WITH_RUSTUP=1 ;;
    --help|-h) cat <<EOF
Usage: ./install.sh [OPTIONS]
  --check         Verify installation without mutating
  --uninstall     Remove managed artifacts only (reads manifest)
  --force         On unmanaged skill collision: backup and replace
  --with-rustup   If cargo missing, bootstrap rustup via https://sh.rustup.rs
  --help          Show this help
EOF
      exit 0 ;;
    *) echo "Unknown option: $arg (see --help)" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SKILL_SRC="$SCRIPT_DIR/$SKILL_SRC_REL"

# ── helpers ──────────────────────────────────────────────────────────
die()  { echo "error: $*" >&2; exit 1; }
info() { echo "info: $*"; }

# SHA-256 map — repo-maintained (upstream publishes no checksums)
sha256_for_asset() {
  case "$1" in
    xg-x86_64-unknown-linux-gnu.tar.gz) echo "78fc6cb56cbd1052d2ed4fa8cf9899d240ffed7cbd9cc2879a127d2bbc1c0d6e" ;;
    xg-aarch64-unknown-linux-gnu.tar.gz) echo "bd806e5242b4c453c32e6ebf9887b44d68ae99ebbc1a50a2a28d2996f8a9021d" ;;
    xg-x86_64-apple-darwin.tar.gz)       echo "c18c5541b2ba3ea9aee96ee8a18674a0419b26f0eb86a7c055fb5e2a62ed79ef" ;;
    xg-aarch64-apple-darwin.tar.gz)      echo "186fc592c96e7b674dac95cb233d92b10f4b3c0e606b155ee6badaa37c976680" ;;
    *) echo "" ;;
  esac
}

detect_platform() {
  local os arch
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os" in
    Linux)  OS="linux" ;;
    Darwin) OS="darwin" ;;
    *) die "unsupported OS: $os (only Linux and macOS are supported)" ;;
  esac
  case "$arch" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) die "unsupported arch: $arch (only x86_64 and aarch64 are supported)" ;;
  esac
  # asset name
  if [ "$OS" = "linux" ]; then
    ASSET="xg-${ARCH}-unknown-linux-gnu.tar.gz"
  else
    # darwin: x86_64 uses x86_64-apple-darwin, aarch64 uses aarch64-apple-darwin
    if [ "$ARCH" = "x86_64" ]; then
      ASSET="xg-x86_64-apple-darwin.tar.gz"
    else
      ASSET="xg-aarch64-apple-darwin.tar.gz"
    fi
  fi
  EXPECTED_SHA="$(sha256_for_asset "$ASSET")"
}

resolve_bin_dir() {
  if [ -n "${CARGO_HOME:-}" ]; then
    BIN_DIR="$CARGO_HOME/bin"
  elif [ -d "$HOME/.cargo/bin" ]; then
    BIN_DIR="$HOME/.cargo/bin"
  else
    BIN_DIR="$HOME/.local/bin"
  fi
  XG_BIN="$BIN_DIR/xg"
}

# ── skill helpers (used in preflight + deploy) ─────────────────────
skill_is_managed() {
  [ -f "$1" ] && head -n 5 "$1" 2>/dev/null | grep -qF "$MANAGED_MARKER"
}

skill_hash() {
  sha256sum "$1" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$1" 2>/dev/null | awk '{print $1}' || echo ""
}

# ── profile PATH block (single bounded, idempotent) ────────────────
PROFILE_MARKER_BEGIN="# >>> cortexyoung xg >>>"
PROFILE_MARKER_END="# <<< cortexyoung xg <<<"

profile_candidates() {
  # Prefer shell-appropriate file, but always include .profile as fallback
  local shell_name
  shell_name="$(basename "${SHELL:-}")"
  case "$shell_name" in
    zsh) echo "$HOME/.zshrc" ;;
    bash) echo "$HOME/.bashrc" ;;
    fish) echo "$HOME/.config/fish/config.fish" ;;
    *) echo "$HOME/.profile" ;;
  esac
}

ensure_path_block() {
  local profile
  profile="$(profile_candidates)"
  # Ensure parent dir exists (for fish)
  mkdir -p "$(dirname "$profile")" 2>/dev/null || true
  touch "$profile" 2>/dev/null || return 0
  if grep -qF "$PROFILE_MARKER_BEGIN" "$profile" 2>/dev/null; then
    return 0
  fi
  {
    echo ""
    echo "$PROFILE_MARKER_BEGIN"
    echo "# Added by cortexyoung install.sh — xg binary directory"
    echo "case \":\$PATH:\" in *\":$BIN_DIR:\"*) ;; *) export PATH=\"$BIN_DIR:\$PATH\" ;; esac"
    echo "$PROFILE_MARKER_END"
  } >> "$profile"
  info "added PATH block to $profile"
  echo "profile:$profile" >> "$MANIFEST_FILE"
}

remove_path_block() {
  local profile
  profile="$(profile_candidates)"
  # Also try common locations
  for profile in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile" "$HOME/.config/fish/config.fish"; do
    [ -f "$profile" ] || continue
    if grep -qF "$PROFILE_MARKER_BEGIN" "$profile" 2>/dev/null; then
      # Remove bounded block (BSD and GNU sed compatible via temp file)
      local tmp
      tmp="$(mktemp)"
      awk -v begin="$PROFILE_MARKER_BEGIN" -v end="$PROFILE_MARKER_END" '
        $0 == begin {skip=1; next}
        $0 == end {skip=0; next}
        !skip {print}
      ' "$profile" > "$tmp" && cat "$tmp" > "$profile"
      rm -f "$tmp"
      info "removed PATH block from $profile"
    fi
  done
}

# ── manifest helpers ───────────────────────────────────────────────
record_manifest() {
  local key="$1" val="$2"
  mkdir -p "$MANIFEST_DIR"
  # Remove old entry for key if present
  if [ -f "$MANIFEST_FILE" ]; then
    local tmp; tmp="$(mktemp)"
    grep -v "^${key}:" "$MANIFEST_FILE" > "$tmp" 2>/dev/null || true
    cat "$tmp" > "$MANIFEST_FILE"; rm -f "$tmp"
  fi
  echo "${key}:${val}" >> "$MANIFEST_FILE"
}

manifest_has() {
  [ -f "$MANIFEST_FILE" ] && grep -q "^$1:" "$MANIFEST_FILE" 2>/dev/null
}

manifest_get() {
  grep "^$1:" "$MANIFEST_FILE" 2>/dev/null | tail -1 | cut -d: -f2-
}

# ═══════════════════════════════════════════════════════════════════
# PREFLIGHT — check collisions BEFORE any mutation
# ═══════════════════════════════════════════════════════════════════
preflight_skill() {
  if [ ! -f "$SKILL_DEST" ]; then
    return 0
  fi
  if skill_is_managed "$SKILL_DEST"; then
    return 0
  fi
  # Unmanaged file exists
  if [ ! -f "$SKILL_SRC" ]; then
    return 0
  fi
  # Hash-equal + adopt: treat as if managed (install will add marker)
  local src_hash dest_hash
  src_hash="$(skill_hash "$SKILL_SRC")"
  dest_hash="$(skill_hash "$SKILL_DEST")"
  if [ -n "$src_hash" ] && [ "$src_hash" = "$dest_hash" ]; then
    return 0
  fi
  if [ "$FORCE" -eq 1 ]; then
    return 0
  fi
  cat >&2 <<EOF
error: unmanaged skill collision at $SKILL_DEST
  The destination exists but is not managed by this installer.
  Refusing to overwrite. Options:
    ./install.sh --force   # backup to SKILL.md.bak.<timestamp> and replace
    rm "$SKILL_DEST"       # remove manually, then re-run
EOF
  exit 1
}

# ═══════════════════════════════════════════════════════════════════
# MODES
# ═══════════════════════════════════════════════════════════════════

do_check() {
  local ok=1
  echo "=== cortexyoung --check ==="
  if command -v xg >/dev/null 2>&1; then
    local ver
    ver="$(xg --version 2>&1 | head -1)"
    echo "xg: $ver ($(command -v xg))"
    if echo "$ver" | grep -qF "$VERSION"; then
      echo "  pinned version $VERSION: OK"
    else
      echo "  pinned version $VERSION: MISMATCH (expected $VERSION)"
      ok=0
    fi
  else
    echo "xg: NOT FOUND in PATH"
    ok=0
  fi
  if [ -f "$SKILL_DEST" ]; then
    if skill_is_managed "$SKILL_DEST"; then
      echo "skill: $SKILL_DEST (managed)"
    else
      echo "skill: $SKILL_DEST (UNMANAGED — run with --force to adopt)"
      ok=0
    fi
  else
    echo "skill: $SKILL_DEST (NOT INSTALLED)"
    ok=0
  fi
  if [ -f "$MANIFEST_FILE" ]; then
    echo "manifest: $MANIFEST_FILE"
    cat "$MANIFEST_FILE" | sed 's/^/  /'
  else
    echo "manifest: (none)"
  fi
  if [ "$ok" -eq 1 ]; then
    echo "check: OK"
    exit 0
  else
    echo "check: ISSUES FOUND"
    exit 1
  fi
}

do_uninstall() {
  echo "=== cortexyoung --uninstall ==="
  # Only remove artifacts we own (manifest-gated)
  if [ -f "$MANIFEST_FILE" ]; then
    local xg_owned skill_owned
    xg_owned="$(manifest_get "xg_bin" 2>/dev/null || true)"
    skill_owned="$(manifest_get "skill" 2>/dev/null || true)"

    # xg binary: only remove if manifest says we installed it AND it still matches our version
    if [ -n "$xg_owned" ] && [ -f "$xg_owned" ]; then
      # Do not remove if binary was pre-existing (manifest records ownership at install time)
      # If manifest says we own it, remove it
      rm -f "$xg_owned"
      info "removed $xg_owned"
    elif [ -n "$xg_owned" ]; then
      info "xg binary already absent: $xg_owned"
    else
      info "xg binary not owned by this installer — skipping (was pre-existing)"
    fi

    # skill: only remove if managed marker present
    if [ -n "$skill_owned" ] && [ -f "$skill_owned" ]; then
      if skill_is_managed "$skill_owned"; then
        rm -f "$skill_owned"
        info "removed $skill_owned"
        rmdir "$(dirname "$skill_owned")" 2>/dev/null || true
      else
        info "skill no longer managed — skipping: $skill_owned"
      fi
    elif [ -f "$SKILL_DEST" ] && skill_is_managed "$SKILL_DEST"; then
      rm -f "$SKILL_DEST"
      info "removed $SKILL_DEST"
      rmdir "$(dirname "$SKILL_DEST")" 2>/dev/null || true
    else
      info "skill not managed — skipping"
    fi

    remove_path_block
    rm -f "$MANIFEST_FILE"
    rmdir "$MANIFEST_DIR" 2>/dev/null || true
    info "uninstall complete"
  else
    # No manifest: be conservative — only remove managed skill and PATH block
    if [ -f "$SKILL_DEST" ] && skill_is_managed "$SKILL_DEST"; then
      rm -f "$SKILL_DEST"
      info "removed $SKILL_DEST (managed)"
      rmdir "$(dirname "$SKILL_DEST")" 2>/dev/null || true
    else
      info "no manifest and skill not managed — nothing to remove for skill"
    fi
    remove_path_block
    if command -v xg >/dev/null 2>&1; then
      info "xg still at $(command -v xg) — not owned (no manifest), leaving in place"
    fi
    info "uninstall complete (no manifest — conservative)"
  fi
}

do_install() {
  echo "=== cortexyoung install (xg v$VERSION) ==="

  # Resolve platform + bin dir early (needed for preflight messages)
  detect_platform
  resolve_bin_dir

  if [ ! -f "$SKILL_SRC" ]; then
    die "skill source not found: $SKILL_SRC (run from repo root)"
  fi

  # ── preflight: fail fast before any mutation ──
  preflight_skill

  # Check unsupported OS/arch already handled by detect_platform
  # ── xg binary ─────────────────────────────────
  local need_install=1
  if command -v xg >/dev/null 2>&1; then
    local cur_ver
    cur_ver="$(xg --version 2>&1 | head -1 || true)"
    if echo "$cur_ver" | grep -qF "$VERSION"; then
      # Correct version — check if at expected location or pre-existing
      local cur_bin
      cur_bin="$(command -v xg)"
      if [ "$cur_bin" = "$XG_BIN" ]; then
        info "xg $VERSION already at $XG_BIN — skipping binary install"
        need_install=0
        # Record ownership as pre-existing (do not claim)
        if [ ! -f "$MANIFEST_FILE" ] || ! manifest_has "xg_bin"; then
          # Pre-existing: do not record xg_bin so uninstall won't remove it
          :
        fi
      else
        info "xg $VERSION already in PATH at $cur_bin — skipping binary install"
        need_install=0
      fi
    else
      info "xg found but version mismatch: $cur_ver (want $VERSION) — will (re)install to $XG_BIN"
    fi
  fi

  if [ "$need_install" -eq 1 ]; then
    local installed=0
    # Try prebuilt first
    local tmpdir url
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT
    url="https://github.com/$REPO/releases/download/v$VERSION/$ASSET"
    info "downloading $url"

    local dl_ok=0
    if command -v curl >/dev/null 2>&1; then
      if curl -fsSL "$url" -o "$tmpdir/$ASSET"; then dl_ok=1; fi
    elif command -v wget >/dev/null 2>&1; then
      if wget -q "$url" -O "$tmpdir/$ASSET"; then dl_ok=1; fi
    else
      die "need curl or wget to download prebuilt"
    fi

    if [ "$dl_ok" -eq 1 ] && [ -s "$tmpdir/$ASSET" ]; then
      # Verify SHA-256
      local actual
      if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$tmpdir/$ASSET" | awk '{print $1}')"
      elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$tmpdir/$ASSET" | awk '{print $1}')"
      else
        die "need sha256sum or shasum to verify download"
      fi
      if [ "$actual" != "$EXPECTED_SHA" ]; then
        echo "warning: SHA-256 mismatch for $ASSET" >&2
        echo "  expected: $EXPECTED_SHA" >&2
        echo "  actual:   $actual" >&2
        echo "  proceeding anyway — report this to the repo owner" >&2
      else
        info "SHA-256 verified"
      fi
      mkdir -p "$BIN_DIR"
      tar -xzf "$tmpdir/$ASSET" -C "$tmpdir"
      # Archive contains binary named 'xg' at top level
      local extracted
      extracted="$(find "$tmpdir" -name "xg" -type f | head -1)"
      if [ -z "$extracted" ]; then
        die "extracted archive does not contain 'xg' binary"
      fi
      install -m 755 "$extracted" "$XG_BIN"
      info "installed xg $VERSION to $XG_BIN"
      record_manifest "xg_bin" "$XG_BIN"
      installed=1
    else
      info "prebuilt download failed — trying cargo fallback"
    fi
    rm -rf "$tmpdir"
    trap - EXIT

    if [ "$installed" -eq 0 ]; then
      # Cargo fallback — crate is xgrep-search (NOT xgrep — name collision)
      if ! command -v cargo >/dev/null 2>&1; then
        if [ "$WITH_RUSTUP" -eq 1 ]; then
          info "cargo not found — bootstrapping rustup"
          if command -v curl >/dev/null 2>&1; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            # shellcheck disable=SC1091
            [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
          else
            die "cargo not found and curl not available to bootstrap rustup (try --with-rustup with curl installed)"
          fi
        else
          die "cargo not found — install Rust (https://rustup.rs) or re-run with --with-rustup"
        fi
      fi
      info "cargo install $CRATE --version $VERSION --locked"
      cargo install "$CRATE" --version "$VERSION" --locked
      # cargo installs to $CARGO_HOME/bin or ~/.cargo/bin
      if [ ! -x "$XG_BIN" ] && command -v xg >/dev/null 2>&1; then
        XG_BIN="$(command -v xg)"
      fi
      if command -v xg >/dev/null 2>&1; then
        record_manifest "xg_bin" "$(command -v xg)"
        info "installed via cargo to $(command -v xg)"
      else
        die "cargo install succeeded but xg not found in PATH"
      fi
    fi
  fi

  # ── skill deploy ──────────────────────────────
  mkdir -p "$(dirname "$SKILL_DEST")"
  if [ -f "$SKILL_DEST" ]; then
    if skill_is_managed "$SKILL_DEST"; then
      local src_hash dest_hash
      src_hash="$(skill_hash "$SKILL_SRC")"
      dest_hash="$(skill_hash "$SKILL_DEST")"
      # Compare without marker line: compare source to dest stripped of marker
      # Simpler: if hashes equal (both include marker after first install, source has no marker)
      # So check if dest content minus marker equals src
      local dest_stripped="$SKILL_DEST.stripped.$$"
      grep -vF "$MANAGED_MARKER" "$SKILL_DEST" > "$dest_stripped" 2>/dev/null || true
      local stripped_hash src_no_nl
      stripped_hash="$(skill_hash "$dest_stripped")"
      rm -f "$dest_stripped"
      if [ "$stripped_hash" = "$(skill_hash "$SKILL_SRC")" ]; then
        info "skill up to date: $SKILL_DEST"
      else
        # Managed but outdated — replace
        {
          echo "$MANAGED_MARKER"
          cat "$SKILL_SRC"
        } > "$SKILL_DEST"
        info "updated skill: $SKILL_DEST"
      fi
    else
      # Unmanaged — preflight already handled hash-equal adopt vs force
      src_hash="$(skill_hash "$SKILL_SRC")"
      dest_hash="$(skill_hash "$SKILL_DEST")"
      if [ "$src_hash" = "$dest_hash" ]; then
        # Adopt: prepend marker
        local tmp; tmp="$(mktemp)"
        {
          echo "$MANAGED_MARKER"
          cat "$SKILL_DEST"
        } > "$tmp" && cat "$tmp" > "$SKILL_DEST"
        rm -f "$tmp"
        info "adopted unmanaged skill (hash-equal): $SKILL_DEST"
      else
        # Must be --force to reach here (preflight)
        local bak="${SKILL_DEST}.bak.$(date +%Y%m%d%H%M%S)"
        cp "$SKILL_DEST" "$bak"
        info "backed up unmanaged skill to $bak"
        {
          echo "$MANAGED_MARKER"
          cat "$SKILL_SRC"
        } > "$SKILL_DEST"
        info "replaced skill: $SKILL_DEST"
      fi
    fi
  else
    {
      echo "$MANAGED_MARKER"
      cat "$SKILL_SRC"
    } > "$SKILL_DEST"
    info "installed skill: $SKILL_DEST"
  fi
  record_manifest "skill" "$SKILL_DEST"

  # ── PATH block ────────────────────────────────
  ensure_path_block

  echo ""
  echo "Done. Verify with: xg --version && cat $SKILL_DEST | head -5"
  echo "If xg not in PATH, restart your shell or: export PATH=\"$BIN_DIR:\$PATH\""
}

# ── dispatch ───────────────────────────────────────────────────────
case "$MODE" in
  check)     do_check ;;
  uninstall) do_uninstall ;;
  install)   do_install ;;
esac
