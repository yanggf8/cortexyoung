#!/usr/bin/env bash
set -euo pipefail
# cortexyoung — xg installer + xgrep skill deploy + cort + ast-grep
# Pinned: xg v0.7.0 from https://github.com/momokun7/xgrep
# Pinned: ast-grep v0.45.2 from https://github.com/ast-grep/ast-grep
# Upstream publishes NO checksums; SHA-256 below is repo-maintained (verified 2026-08-26).
# Usage: ./install.sh [--check] [--uninstall] [--force] [--with-rustup] [--with-xgrep]

VERSION="0.7.0"
REPO="momokun7/xgrep"
CRATE="xgrep-search"
AST_GREP_VERSION="0.45.2"
AST_GREP_REPO="ast-grep/ast-grep"
AST_GREP_CRATE="ast-grep"
CORT_VERSION="0.1.0"
MANAGED_MARKER="# managed by cortexyoung install.sh"
MANIFEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung"
MANIFEST_FILE="$MANIFEST_DIR/manifest"
SKILL_SRC_REL="skills/xgrep/SKILL.md"
SKILL_DEST="$HOME/.claude/skills/xgrep/SKILL.md"
CORT_HOME="$MANIFEST_DIR/cort"
AST_GREP_SKILL_SRC_REL="skills/ast-grep/SKILL.md"
AST_GREP_SKILL_DEST="$HOME/.claude/skills/ast-grep/SKILL.md"
WITH_XGREP=0

sha256_for_ast_grep_asset() {
  case "$1" in
    app-x86_64-unknown-linux-gnu.zip)  echo "67aff72dd2994bf152fcc3a8a09cf93b13193abe59f39393095167c729af2015" ;;
    app-aarch64-unknown-linux-gnu.zip) echo "e67ee2f5928b4d77a472114edf6e227d90fefe22fa47e7a78db187c55d206564" ;;
    app-x86_64-apple-darwin.zip)       echo "037e5b4a9aed2ba03a2b4710e4fe3439d5d1154d1266d5e8f9f6df7452169181" ;;
    app-aarch64-apple-darwin.zip)      echo "1fc21214234bf6f5a3f841d5b2493a4fc4b6087f69b055c9ad5f94f77c0ab76e" ;;
    *) echo "" ;;
  esac
}

FORCE=0; WITH_RUSTUP=0; MODE="install"

for arg in "$@"; do
  case "$arg" in
    --check)     MODE="check" ;;
    --uninstall) MODE="uninstall" ;;
    --force)     FORCE=1 ;;
    --with-rustup) WITH_RUSTUP=1 ;;
    --with-xgrep) WITH_XGREP=1 ;;
    --help|-h) cat <<EOF
Usage: ./install.sh [OPTIONS]
  --check         Verify installation without mutating
  --uninstall     Remove managed artifacts only (reads manifest)
  --force         On unmanaged skill collision: backup and replace
  --with-rustup   If cargo missing, bootstrap rustup via https://sh.rustup.rs
  --with-xgrep    Also install xg (opt-in; default is cort + ast-grep only)
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

# Unified download helper — tries curl then wget; returns 0 on success.
download() {
  local url="$1" dest="$2"
  if command -v curl >/dev/null 2>&1; then
    if curl -fsSL "$url" -o "$dest"; then return 0; fi
  fi
  if command -v wget >/dev/null 2>&1; then
    if wget -q "$url" -O "$dest"; then return 0; fi
  fi
  return 1
}

# Fail-closed SHA-256 verification shared by xg and ast-grep.
verify_sha() {
  local file="$1" expected="$2"
  [ -n "$expected" ] || die "no checksum on record for $file"
  local actual
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  else
    die "need sha256sum or shasum to verify download"
  fi
  if [ "$actual" != "$expected" ]; then
    echo "error: SHA-256 mismatch for $file" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    die "refusing to install an unverified binary"
  fi
  info "SHA-256 verified"
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
  # asset name for xg
  if [ "$OS" = "linux" ]; then
    ASSET="xg-${ARCH}-unknown-linux-gnu.tar.gz"
  else
    if [ "$ARCH" = "x86_64" ]; then
      ASSET="xg-x86_64-apple-darwin.tar.gz"
    else
      ASSET="xg-aarch64-apple-darwin.tar.gz"
    fi
  fi
  EXPECTED_SHA="$(sha256_for_asset "$ASSET")"
  # TARGET maps directly to Rust target triple suffix
  if [ "$OS" = "linux" ]; then
    TARGET="${ARCH}-unknown-linux-gnu"
  else
    TARGET="${ARCH}-apple-darwin"
  fi
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
  for profile in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile" "$HOME/.config/fish/config.fish"; do
    [ -f "$profile" ] || continue
    if grep -qF "$PROFILE_MARKER_BEGIN" "$profile" 2>/dev/null; then
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

migrate_manifest_v2() {
  [ -f "$MANIFEST_FILE" ] || { record_manifest "manifest_version" "2"; return 0; }
  if [ "$(manifest_get manifest_version)" = "2" ]; then return 0; fi
  local old_xg old_skill
  old_xg="$(manifest_get xg_bin || true)"
  old_skill="$(manifest_get skill || true)"
  [ -n "$old_xg" ] && record_manifest "legacy_xg_bin" "$old_xg"
  [ -n "$old_skill" ] && record_manifest "skill_xgrep" "$old_skill"
  local tmp; tmp="$(mktemp)"
  grep -v '^xg_bin:' "$MANIFEST_FILE" | grep -v '^skill:' > "$tmp" || true
  cat "$tmp" > "$MANIFEST_FILE"; rm -f "$tmp"
  record_manifest "manifest_version" "2"
  info "migrated manifest to v2 (xg_bin -> legacy_xg_bin, skill -> skill_xgrep)"
}

# ═══════════════════════════════════════════════════════════════════
# PREFLIGHT — check collisions BEFORE any mutation (two-skill variant)
# ═══════════════════════════════════════════════════════════════════
preflight_skill_at() {
  local src="$1" dest="$2"
  if [ ! -f "$dest" ]; then
    return 0
  fi
  if skill_is_managed "$dest"; then
    return 0
  fi
  if [ ! -f "$src" ]; then
    return 0
  fi
  local src_hash dest_hash
  src_hash="$(skill_hash "$src")"
  dest_hash="$(skill_hash "$dest")"
  if [ -n "$src_hash" ] && [ "$src_hash" = "$dest_hash" ]; then
    return 0
  fi
  if [ "$FORCE" -eq 1 ]; then
    return 0
  fi
  cat >&2 <<EOF
error: unmanaged skill collision at $dest
  The destination exists but is not managed by this installer.
  Refusing to overwrite. Options:
    ./install.sh --force   # backup to SKILL.md.bak.<timestamp> and replace
    rm "$dest"       # remove manually, then re-run
EOF
  exit 1
}

# legacy single-skill wrapper for backwards compat (no longer used in install path)
preflight_skill() {
  preflight_skill_at "$SKILL_SRC" "$SKILL_DEST"
}

deploy_skill_at() {
  local src="$1" dest="$2" key="$3"
  if [ ! -f "$src" ]; then
    info "skill source not found: $src — skipping $key"
    return 0
  fi
  mkdir -p "$(dirname "$dest")"
  if [ -f "$dest" ]; then
    if skill_is_managed "$dest"; then
      local dest_stripped
      dest_stripped="$(mktemp)"
      grep -vF "$MANAGED_MARKER" "$dest" > "$dest_stripped" 2>/dev/null || true
      local stripped_hash src_hash
      stripped_hash="$(skill_hash "$dest_stripped")"
      src_hash="$(skill_hash "$src")"
      rm -f "$dest_stripped"
      if [ "$stripped_hash" = "$src_hash" ]; then
        info "skill up to date: $dest"
      else
        {
          echo "$MANAGED_MARKER"
          cat "$src"
        } > "$dest"
        info "updated skill: $dest"
      fi
    else
      local src_hash dest_hash
      src_hash="$(skill_hash "$src")"
      dest_hash="$(skill_hash "$dest")"
      if [ "$src_hash" = "$dest_hash" ]; then
        local tmp; tmp="$(mktemp)"
        {
          echo "$MANAGED_MARKER"
          cat "$dest"
        } > "$tmp" && cat "$tmp" > "$dest"
        rm -f "$tmp"
        info "adopted unmanaged skill (hash-equal): $dest"
      else
        local bak="${dest}.bak.$(date +%Y%m%d%H%M%S)"
        cp "$dest" "$bak"
        info "backed up unmanaged skill to $bak"
        {
          echo "$MANAGED_MARKER"
          cat "$src"
        } > "$dest"
        info "replaced skill: $dest"
      fi
    fi
  else
    {
      echo "$MANAGED_MARKER"
      cat "$src"
    } > "$dest"
    info "installed skill: $dest"
  fi
  record_manifest "$key" "$dest"
}

# ═══════════════════════════════════════════════════════════════════
# install helpers: xg / ast-grep / cort
# ═══════════════════════════════════════════════════════════════════
install_xg() {
  local need_install=1
  if command -v xg >/dev/null 2>&1; then
    local cur_ver
    cur_ver="$(xg --version 2>&1 | head -1 || true)"
    if echo "$cur_ver" | grep -qF "$VERSION"; then
      local cur_bin
      cur_bin="$(command -v xg)"
      if [ "$cur_bin" = "$XG_BIN" ]; then
        info "xg $VERSION already at $XG_BIN — skipping binary install"
        need_install=0
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
    local tmpdir url
    tmpdir="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf \"$tmpdir\"" EXIT
    url="https://github.com/$REPO/releases/download/v$VERSION/$ASSET"
    info "downloading $url"

    local dl_ok=0
    local dl_dest="$tmpdir/$ASSET"
    if download "$url" "$dl_dest"; then
      if [ -s "$dl_dest" ]; then dl_ok=1; fi
    fi

    if [ "$dl_ok" -eq 1 ]; then
      verify_sha "$dl_dest" "$EXPECTED_SHA"
      mkdir -p "$BIN_DIR"
      tar -xzf "$dl_dest" -C "$tmpdir"
      local extracted
      extracted="$(find "$tmpdir" -name "xg" -type f | head -1)"
      if [ -z "$extracted" ]; then
        die "extracted archive does not contain 'xg' binary"
      fi
      install -m 755 "$extracted" "$XG_BIN"
      info "installed xg $VERSION to $XG_BIN"
      record_manifest "legacy_xg_bin" "$XG_BIN"
      installed=1
    else
      info "prebuilt download failed — trying cargo fallback"
    fi
    rm -rf "$tmpdir"
    trap - EXIT

    if [ "$installed" -eq 0 ]; then
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
      if [ ! -x "$XG_BIN" ] && command -v xg >/dev/null 2>&1; then
        XG_BIN="$(command -v xg)"
      fi
      if command -v xg >/dev/null 2>&1; then
        record_manifest "legacy_xg_bin" "$(command -v xg)"
        info "installed via cargo to $(command -v xg)"
      else
        die "cargo install succeeded but xg not found in PATH"
      fi
    fi
  fi
}

install_ast_grep() {
  if command -v ast-grep >/dev/null 2>&1 \
     && [ "$(ast-grep --version | awk '{print $2}')" = "$AST_GREP_VERSION" ]; then
    info "ast-grep $AST_GREP_VERSION already present"
    return 0
  fi
  if command -v sg >/dev/null 2>&1 && ! sg --version 2>/dev/null | grep -q '^ast-grep '; then
    info "ignoring unrelated 'sg' on PATH (not ast-grep)"
  fi
  local asset="app-${TARGET}.zip"
  local url="https://github.com/${AST_GREP_REPO}/releases/download/${AST_GREP_VERSION}/${asset}"
  local expected; expected="$(sha256_for_ast_grep_asset "$asset")"
  [ -n "$expected" ] || die "no checksum on record for $asset"
  local tmpdir; tmpdir="$(mktemp -d)"
  if download "$url" "$tmpdir/$asset"; then
    verify_sha "$tmpdir/$asset" "$expected"
    command -v unzip >/dev/null 2>&1 || die "need unzip to extract $asset"
    unzip -q "$tmpdir/$asset" -d "$tmpdir"
    [ -x "$tmpdir/ast-grep" ] || die "$asset did not contain an ast-grep binary"
    mkdir -p "$BIN_DIR"
    install -m 755 "$tmpdir/ast-grep" "$BIN_DIR/ast-grep"
    record_manifest "ast_grep_bin" "$BIN_DIR/ast-grep"
  else
    command -v cargo >/dev/null 2>&1 || die "download failed and cargo not found; ast-grep $AST_GREP_VERSION needs Rust 1.88+"
    cargo install "$AST_GREP_CRATE" --version "$AST_GREP_VERSION" --locked \
      || die "cargo install ast-grep failed; ast-grep $AST_GREP_VERSION requires Rust 1.88+"
    record_manifest "ast_grep_bin" "$(command -v ast-grep)"
  fi
  rm -rf "$tmpdir"
  [ "$(ast-grep --version | awk '{print $2}')" = "$AST_GREP_VERSION" ] \
    || die "ast-grep version mismatch after install"
}

install_cort() {
  command -v node >/dev/null 2>&1 || die "cort needs Node.js >= 22"
  local major; major="$(node -p 'process.versions.node.split(".")[0]')"
  [ "$major" -ge 22 ] || die "cort needs Node.js >= 22 (found $(node -v))"

  rm -rf "$CORT_HOME"
  mkdir -p "$CORT_HOME"
  cp -R "$SCRIPT_DIR/bin" "$SCRIPT_DIR/src" "$SCRIPT_DIR/package.json" "$CORT_HOME/"
  [ -f "$SCRIPT_DIR/package-lock.json" ] && cp "$SCRIPT_DIR/package-lock.json" "$CORT_HOME/"
  ( cd "$CORT_HOME" && npm ci --omit=dev --silent ) || die "npm ci failed in $CORT_HOME"

  mkdir -p "$BIN_DIR"
  local shim="$BIN_DIR/cort"
  cat > "$shim.tmp" <<SHIM
#!/usr/bin/env bash
if [ "\$1" = "--version" ]; then echo "cort $CORT_VERSION"; exit 0; fi
exec node "$CORT_HOME/bin/cort.js" "\$@"
SHIM
  chmod 755 "$shim.tmp"
  mv "$shim.tmp" "$shim"
  record_manifest "cort_bin" "$shim"
  "$shim" status >/dev/null 2>&1 || true
  info "installed cort $CORT_VERSION -> $shim"
}

# ═══════════════════════════════════════════════════════════════════
# MODES
# ═══════════════════════════════════════════════════════════════════

do_check() {
  local ok=1
  echo "=== cortexyoung --check ==="
  # cort
  if command -v cort >/dev/null 2>&1; then
    local ver
    ver="$(cort --version 2>&1 | head -1 || true)"
    echo "cort: $ver ($(command -v cort))"
    if echo "$ver" | grep -qF "$CORT_VERSION"; then
      echo "  pinned version $CORT_VERSION: OK"
    else
      echo "  pinned version $CORT_VERSION: MISMATCH (expected $CORT_VERSION)"
      ok=0
    fi
  else
    echo "cort: NOT FOUND in PATH"
    ok=0
  fi
  # ast-grep
  if command -v ast-grep >/dev/null 2>&1; then
    local ver
    ver="$(ast-grep --version 2>&1 | head -1)"
    echo "ast-grep: $ver ($(command -v ast-grep))"
    if echo "$ver" | grep -qF "$AST_GREP_VERSION"; then
      echo "  pinned version $AST_GREP_VERSION: OK"
    else
      echo "  pinned version $AST_GREP_VERSION: MISMATCH (expected $AST_GREP_VERSION)"
      ok=0
    fi
  else
    echo "ast-grep: NOT FOUND in PATH"
    ok=0
  fi
  # xg optional
  if command -v xg >/dev/null 2>&1; then
    local ver
    ver="$(xg --version 2>&1 | head -1)"
    echo "xg: $ver ($(command -v xg))"
    if echo "$ver" | grep -qF "$VERSION"; then
      echo "  pinned version $VERSION: OK"
    else
      echo "  pinned version $VERSION: MISMATCH (expected $VERSION)"
      # xg mismatch is not fatal for default install (opt-in)
      if [ "$WITH_XGREP" -eq 1 ]; then ok=0; fi
    fi
  else
    if [ "$WITH_XGREP" -eq 1 ]; then
      echo "xg: NOT FOUND in PATH (required with --with-xgrep)"
      ok=0
    else
      echo "xg: NOT FOUND in PATH (opt-in via --with-xgrep)"
    fi
  fi
  if [ -f "$SKILL_DEST" ]; then
    if skill_is_managed "$SKILL_DEST"; then
      echo "skill: $SKILL_DEST (managed)"
    else
      echo "skill: $SKILL_DEST (UNMANAGED — run with --force to adopt)"
    fi
  else
    echo "skill: $SKILL_DEST (NOT INSTALLED)"
  fi
  if [ -f "$AST_GREP_SKILL_DEST" ]; then
    if skill_is_managed "$AST_GREP_SKILL_DEST"; then
      echo "skill_ast_grep: $AST_GREP_SKILL_DEST (managed)"
    else
      echo "skill_ast_grep: $AST_GREP_SKILL_DEST (UNMANAGED)"
      ok=0
    fi
  else
    echo "skill_ast_grep: $AST_GREP_SKILL_DEST (NOT INSTALLED)"
    ok=0
  fi
  if [ -f "$MANIFEST_FILE" ]; then
    echo "manifest: $MANIFEST_FILE"
    cat "$MANIFEST_FILE" | sed 's/^/  /'
    local mv
    mv="$(manifest_get manifest_version 2>/dev/null || true)"
    if [ "$mv" = "2" ]; then
      echo "  manifest_version 2: OK"
    else
      echo "  manifest_version: MISMATCH (expected 2, got ${mv:-none})"
      ok=0
    fi
  else
    echo "manifest: (none)"
    ok=0
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
  if [ -f "$MANIFEST_FILE" ]; then
    local cort_owned ag_owned xg_owned skill_ag skill_xg
    cort_owned="$(manifest_get cort_bin || true)"
    ag_owned="$(manifest_get ast_grep_bin || true)"
    xg_owned="$(manifest_get legacy_xg_bin || true)"
    skill_ag="$(manifest_get skill_ast_grep || true)"
    skill_xg="$(manifest_get skill_xgrep || true)"

    if [ -n "$cort_owned" ] && [ -f "$cort_owned" ]; then
      rm -f "$cort_owned"
      info "removed $cort_owned"
    elif [ -n "$cort_owned" ]; then
      info "cort binary already absent: $cort_owned"
    else
      info "cort binary not owned by this installer — skipping"
    fi
    if [ -n "$CORT_HOME" ] && [ -d "$CORT_HOME" ]; then
      rm -rf "$CORT_HOME"
      info "removed $CORT_HOME"
    fi

    if [ -n "$ag_owned" ] && [ -f "$ag_owned" ]; then
      rm -f "$ag_owned"
      info "removed $ag_owned"
    elif [ -n "$ag_owned" ]; then
      info "ast-grep binary already absent: $ag_owned"
    else
      info "ast-grep binary not owned by this installer — skipping"
    fi

    if [ -n "$xg_owned" ] && [ -f "$xg_owned" ]; then
      rm -f "$xg_owned"
      info "removed $xg_owned"
    elif [ -n "$xg_owned" ]; then
      info "xg binary already absent: $xg_owned"
    else
      info "xg binary not owned by this installer — skipping (was pre-existing)"
    fi

    if [ -n "$skill_ag" ] && [ -f "$skill_ag" ]; then
      if skill_is_managed "$skill_ag"; then
        rm -f "$skill_ag"
        info "removed $skill_ag"
        rmdir "$(dirname "$skill_ag")" 2>/dev/null || true
      else
        info "skill_ast_grep no longer managed — skipping: $skill_ag"
      fi
    elif [ -f "$AST_GREP_SKILL_DEST" ] && skill_is_managed "$AST_GREP_SKILL_DEST"; then
      rm -f "$AST_GREP_SKILL_DEST"
      info "removed $AST_GREP_SKILL_DEST"
      rmdir "$(dirname "$AST_GREP_SKILL_DEST")" 2>/dev/null || true
    else
      info "skill_ast_grep not managed — skipping"
    fi

    if [ -n "$skill_xg" ] && [ -f "$skill_xg" ]; then
      if skill_is_managed "$skill_xg"; then
        rm -f "$skill_xg"
        info "removed $skill_xg"
        rmdir "$(dirname "$skill_xg")" 2>/dev/null || true
      else
        info "skill_xgrep no longer managed — skipping: $skill_xg"
      fi
    elif [ -f "$SKILL_DEST" ] && skill_is_managed "$SKILL_DEST"; then
      rm -f "$SKILL_DEST"
      info "removed $SKILL_DEST"
      rmdir "$(dirname "$SKILL_DEST")" 2>/dev/null || true
    else
      info "skill_xgrep not managed — skipping"
    fi

    remove_path_block
    rm -f "$MANIFEST_FILE"
    rmdir "$MANIFEST_DIR" 2>/dev/null || true
    info "uninstall complete"
  else
    if [ -f "$SKILL_DEST" ] && skill_is_managed "$SKILL_DEST"; then
      rm -f "$SKILL_DEST"
      info "removed $SKILL_DEST (managed)"
      rmdir "$(dirname "$SKILL_DEST")" 2>/dev/null || true
    else
      info "no manifest and skill not managed — nothing to remove for skill"
    fi
    if [ -f "$AST_GREP_SKILL_DEST" ] && skill_is_managed "$AST_GREP_SKILL_DEST"; then
      rm -f "$AST_GREP_SKILL_DEST"
      info "removed $AST_GREP_SKILL_DEST (managed)"
      rmdir "$(dirname "$AST_GREP_SKILL_DEST")" 2>/dev/null || true
    fi
    remove_path_block
    if [ -d "$CORT_HOME" ]; then
      rm -rf "$CORT_HOME"
      info "removed $CORT_HOME (no manifest — conservative)"
    fi
    if command -v cort >/dev/null 2>&1; then
      info "cort still at $(command -v cort) — not owned (no manifest), leaving in place"
    fi
    if command -v ast-grep >/dev/null 2>&1; then
      info "ast-grep still at $(command -v ast-grep) — not owned (no manifest), leaving in place"
    fi
    if command -v xg >/dev/null 2>&1; then
      info "xg still at $(command -v xg) — not owned (no manifest), leaving in place"
    fi
    info "uninstall complete (no manifest — conservative)"
  fi
}

do_install() {
  echo "=== cortexyoung install (cort v$CORT_VERSION, ast-grep v$AST_GREP_VERSION) ==="

  migrate_manifest_v2
  detect_platform
  resolve_bin_dir

  # Preflight both skills before any mutation — atomic rollback
  preflight_skill_at "$SCRIPT_DIR/$AST_GREP_SKILL_SRC_REL" "$AST_GREP_SKILL_DEST"
  if [ "$WITH_XGREP" -eq 1 ]; then
    if [ ! -f "$SCRIPT_DIR/$SKILL_SRC_REL" ]; then
      die "skill source not found: $SCRIPT_DIR/$SKILL_SRC_REL (run from repo root)"
    fi
    preflight_skill_at "$SCRIPT_DIR/$SKILL_SRC_REL" "$SKILL_DEST"
  fi

  install_ast_grep
  install_cort
  if [ "$WITH_XGREP" -eq 1 ]; then
    install_xg
  fi
  deploy_skill_at "$SCRIPT_DIR/$AST_GREP_SKILL_SRC_REL" "$AST_GREP_SKILL_DEST" "skill_ast_grep"
  if [ "$WITH_XGREP" -eq 1 ]; then
    deploy_skill_at "$SCRIPT_DIR/$SKILL_SRC_REL" "$SKILL_DEST" "skill_xgrep"
  fi
  ensure_path_block
  record_manifest "manifest_version" "2"

  echo ""
  echo "Done. Verify with: cort --version && ast-grep --version && cat $AST_GREP_SKILL_DEST | head -5"
  if [ "$WITH_XGREP" -eq 1 ]; then
    echo "Also: xg --version && cat $SKILL_DEST | head -5"
  fi
  echo "If cort not in PATH, restart your shell or: export PATH=\"$BIN_DIR:\$PATH\""
}

# ── dispatch ───────────────────────────────────────────────────────
case "$MODE" in
  check)     do_check ;;
  uninstall) do_uninstall ;;
  install)   do_install ;;
esac
