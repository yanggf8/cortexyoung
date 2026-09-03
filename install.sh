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
# Ownership text, written into a stamp file NEXT TO a deployed SKILL.md. It is deliberately not
# written into SKILL.md itself (F-19): that document is input to two third-party frontmatter
# parsers, so every byte the installer inserted there was a byte it had to subtract again before
# comparing, and a comment inside the YAML block is bookkeeping in a format we do not own.
MANAGED_SIGNATURE="managed by cortexyoung install.sh"
MANIFEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/cortexyoung"
MANIFEST_FILE="$MANIFEST_DIR/manifest"
DEPLOY_LOG="$MANIFEST_DIR/deploy.log"
SKILL_SRC_REL="skills/xgrep/SKILL.md"
SKILL_DEST="$HOME/.claude/skills/xgrep/SKILL.md"
CORT_HOME="$MANIFEST_DIR/cort"
AST_GREP_SKILL_SRC_REL="skills/ast-grep/SKILL.md"
AST_GREP_SKILL_DEST="${CLAUDE_SKILL_HOME:-$HOME/.claude}/skills/ast-grep/SKILL.md"
# The same routing guidance is deployed for Codex (it reads $CODEX_HOME/skills, default
# ~/.codex/skills). One source of truth in the repo, two agent homes — an agent that never sees the
# skill routes ordinary lookups into the graph and re-reads whole files.
CODEX_SKILL_DEST="${CODEX_HOME:-$HOME/.codex}/skills/ast-grep/SKILL.md"
# The PreToolUse hook ships with the skill rather than being a second thing to wire per agent home.
# The skill is prospective ("before you rename X, run impact") and measured over the sessions that
# carried it, it never fired: 409 searches, zero cort calls. The hook is the retrospective half of
# the same routing, and a routing half that has to be installed by hand is a routing half that is
# not installed. `cort hook-install` owns the JSON merge -- see rust/src/settings.rs for why that is
# not a jq pipeline in here. Grok reads this same file for Claude Code compatibility and needs no
# entry of its own (docs/2026-09-02-hook-wiring-correction.md §6).
HOOK_SETTINGS="${CLAUDE_SKILL_HOME:-$HOME/.claude}/settings.json"
# Codex loads a PreToolUse hook only from `[[hooks.PreToolUse]]` in this TOML file -- neither of the
# JSON locations it might plausibly read (~/.codex/hooks/hooks.json, ~/.codex/hooks.json) is ever
# consulted (docs/2026-09-02-hook-wiring-correction.md §12). `rust/src/settings_toml.rs` owns this
# merge for the same reason `rust/src/settings.rs` owns the JSON one. Deployed unconditionally, same
# as CODEX_SKILL_DEST above: harmless if Codex is not installed on this machine, and the alternative
# is the exact failure this file is about -- a route wired only when someone remembers Codex exists.
CODEX_HOOK_SETTINGS="${CODEX_HOME:-$HOME/.codex}/config.toml"
# Kimi's own config file, and the third dialect: a flat top-level `[[hooks]]` array, not Codex's
# nested groups. Both files are called config.toml, which is why `hook-install` now takes an
# explicit --format instead of reading the extension.
KIMI_HOOK_SETTINGS="${KIMI_CODE_HOME:-$HOME/.kimi-code}/config.toml"
WITH_HOOK=1
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
    --no-hook)   WITH_HOOK=0 ;;
    --help|-h) cat <<EOF
Usage: ./install.sh [OPTIONS]
  --check         Verify installation without mutating
  --uninstall     Remove managed artifacts only (reads manifest)
  --force         On unmanaged skill collision: backup and replace
  --with-rustup   If cargo missing, bootstrap rustup via https://sh.rustup.rs
  --with-xgrep    Also install xg (opt-in; default is cort + ast-grep only)
  --no-hook       Do not wire the PreToolUse hook into settings.json
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
# The stamp file lives in the same directory as the SKILL.md it claims, one directory per skill.
MANAGED_STAMP_NAME=".cortexyoung-managed"

skill_stamp_for() {
  printf '%s/%s' "$(dirname "$1")" "$MANAGED_STAMP_NAME"
}

# skill_is_managed: is this SKILL.md ours? Two answers, in this order:
#   1. the file still carries the in-file marker written by install.sh before F-19 (above the
#      fence, or as a YAML comment inside it) -- it is ours and deploy_skill_at repairs the shape;
#   2. the stamp beside it records the SHA-256 of the file we wrote. Ownership tracks content, so a
#      hand-edit of a deployed skill is an unmanaged collision, not a licence to overwrite it.
skill_is_managed() {
  if [ ! -f "$1" ]; then
    return 1
  fi
  if grep -qF "$MANAGED_SIGNATURE" "$1"; then
    return 0
  fi
  local stamp
  stamp="$(skill_stamp_for "$1")"
  if [ ! -f "$stamp" ]; then
    return 1
  fi
  grep -qF "skill_sha256:$(skill_hash "$1")" "$stamp"
}

# ensure_skill_stamp: claim "$1", a SKILL.md we just wrote, in the stamp file beside it.
ensure_skill_stamp() {
  printf '%s\nskill_sha256:%s\n' "$MANAGED_SIGNATURE" "$(skill_hash "$1")" > "$(skill_stamp_for "$1")"
}

# write_skill: publish "$1" to "$2" byte-for-byte, then claim it. Nothing is inserted into the
# document -- what lands in an agent home directory is exactly what is in skills/<name>/SKILL.md.
write_skill() {
  cat "$1" > "$2"
  ensure_skill_stamp "$2"
}

# skill_frontmatter_intact: the shape both loaders require. Codex skips a skill whose fence does not
# open at byte 0 (measured with `codex debug prompt-input`: text above the fence makes the whole
# skill invisible, a comment inside it does not), and Claude Code 2.1.251 matches a fence anchored
# to line 1 and, on no match, returns an empty frontmatter with no diagnostic at all. Neither
# format asks for a comment inside the block, so install.sh treats one as what it is: a file
# something else has been editing. Indented continuation lines are legal YAML and are left alone.
skill_frontmatter_intact() {
  awk '
    NR == 1 { if ($0 != "---") exit 1; next }
    $0 == "---" { closed = 1; exit 0 }
    /^[ \t]*#/ { exit 1 }
    END { if (!closed) exit 1 }
  ' "$1" 2>/dev/null
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
# Which bytes were in an agent home, and from when. Nothing else can answer that: the stamp records
# the sha but no time, the manifest records neither, and a redeploy overwrites the mtime. Until this
# log the only way to date an exposure was to grep the frontmatter description out of session
# transcripts -- which works only when the description itself changed. On 2026-09-01 a body-only edit
# moved the sha and left the description byte-identical, so that method went blind on the same day it
# was first used. Append-only, one line per actual change, `<iso8601>\t<sha>\t<dest>`; `absent` is a
# real sha value here, meaning the file was removed, because "no line" and "removed" are different
# claims. Cross it with a session's start time to attribute that session to a version.
record_deploy() {
  local dest="$1" sha="$2" last=""
  mkdir -p "$MANIFEST_DIR"
  if [ -f "$DEPLOY_LOG" ]; then
    last="$(awk -F'\t' -v d="$dest" '$3 == d { s = $2 } END { print s }' "$DEPLOY_LOG")"
  fi
  if [ "$last" != "$sha" ]; then
    printf '%s\t%s\t%s\n' "$(date -Iseconds)" "$sha" "$dest" >> "$DEPLOY_LOG"
  fi
}

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
  # A source no loader can read is worth failing on before any mutation, not after a "success".
  if [ -f "$src" ] && ! skill_frontmatter_intact "$src"; then
    die "skill source has no usable YAML frontmatter (the fence must open on line 1 and close before the body): $src"
  fi
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
  The destination is not a file this installer wrote and still owns: it was never installed here,
  or it has been edited since. Ownership is the SHA-256 in $MANAGED_STAMP_NAME beside the file.
  Edit skills/<name>/SKILL.md in the repo instead, so both agent homes get the same change.
  Refusing to overwrite. Options:
    ./install.sh --force   # backup to SKILL.md.bak.<timestamp> and replace
    rm "$dest"       # remove manually, then re-run
EOF
  exit 1
}

# remove_managed_skill_at: delete a skill this installer owns, and nothing else. An unmanaged file
# at the same path is reported and left alone — uninstall must never eat someone else's config.
# The stamp is removed with the document: an orphan stamp in a leftover directory would claim
# ownership of whatever SKILL.md someone writes there next.
remove_managed_skill_at() {
  local path="$1" label="$2"
  if [ -f "$path" ] && skill_is_managed "$path"; then
    rm -f "$path" "$(skill_stamp_for "$path")"
    info "removed $path"
    rmdir "$(dirname "$path")" 2>/dev/null || true
  elif [ -f "$path" ]; then
    info "$label not managed — skipping: $path"
  else
    info "$label not present — nothing to remove"
  fi
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
  local src_hash dest_hash
  src_hash="$(skill_hash "$src")"
  if [ -f "$dest" ]; then
    dest_hash="$(skill_hash "$dest")"
    if skill_is_managed "$dest"; then
      if [ "$dest_hash" = "$src_hash" ]; then
        ensure_skill_stamp "$dest"
        info "skill up to date: $dest"
      elif grep -qF "$MANAGED_SIGNATURE" "$dest"; then
        # Written by an older installer, which put the marker inside the document. Identical apart
        # from that one line, so repair the shape instead of reporting a clean run.
        write_skill "$src" "$dest"
        info "repaired skill: $dest (installer marker moved out of the document)"
      else
        write_skill "$src" "$dest"
        info "updated skill: $dest"
      fi
    else
      if [ "$src_hash" = "$dest_hash" ]; then
        ensure_skill_stamp "$dest"
        info "adopted unmanaged skill (hash-equal): $dest"
      else
        local bak
        bak="${dest}.bak.$(date +%Y%m%d%H%M%S)"
        cp "$dest" "$bak"
        info "backed up unmanaged skill to $bak"
        write_skill "$src" "$dest"
        info "replaced skill: $dest"
      fi
    fi
  else
    write_skill "$src" "$dest"
    info "installed skill: $dest"
  fi
  record_manifest "$key" "$dest"
  record_deploy "$dest" "$src_hash"
}

# ── PreToolUse hook (deployed with the skill, same run) ────────────
# Delegated to `cort hook-install`: preserving the hooks the user already has, staying idempotent
# across reinstalls, recognising its own entry after the binary moves and refusing a settings.json
# it cannot parse are all logic, and logic does not go in bash here. Failure is reported, never
# fatal -- a settings file we could not merge must not cost the user a working binary.
# Read-only: never writes. Shared by `--check` for both the Claude/Grok and the Codex hook, so the
# "wired to a different binary" and "not wired" logic exists exactly once regardless of which
# settings file is being asked about.
# Read every entry from `hook-install --all --status --lean` and apply the same three tests
# `check_hook_at` applies to one: wired at all, wired to the binary we manage, and (Codex only)
# actually reviewed. The tests live here rather than in the binary because "is this the cort this
# installer deployed" is a question about the install, not about the settings file.
check_all_hooks() {
  local managed_cort="$1"
  local harness ev outcome settings detail label out
  if ! out="$("$managed_cort" hook-install --all --status --lean 2>&1)"; then
    if printf '%s' "$out" | grep -q '"unknown_command"'; then
      echo "hook: installed cort predates hook-install --all — re-run ./install.sh to redeploy the binary"
    else
      echo "hook: could not query hook state — $out"
    fi
    ok=0
    return 0
  fi
  while IFS=$'\t' read -r harness ev outcome settings detail command; do
    [ -n "$harness" ] || continue
    label="$harness/$ev"
    if [ "$outcome" != "wired" ]; then
      echo "$label: $settings (NOT WIRED — re-run ./install.sh)"
      if [ "$WITH_HOOK" -eq 1 ]; then ok=0; fi
      continue
    fi
    # Wired is not enough: it has to be wired to the binary we just checked. A command naming a
    # different cort is a live hook nobody here has verified. Compared per entry, against the
    # command that entry actually carries -- an aggregate "some entry names it" would pass a file
    # where five of six point somewhere else.
    case "$command" in
      "$managed_cort "*) : ;;
      *)
        echo "$label: $settings (WIRED TO A DIFFERENT BINARY — re-run ./install.sh)"
        echo "  wired: $command"
        echo "  managed: $managed_cort"
        if [ "$WITH_HOOK" -eq 1 ]; then ok=0; fi
        continue
        ;;
    esac
    # Codex will not run a hook it has not been shown, so an unreviewed entry sits in config.toml
    # firing nothing. Reported, not failed: this hook deploys unconditionally, so "wired and never
    # reviewed" is also the resting state of a machine with no Codex on it. Trust is the user's to
    # give; naming it is ours.
    [ "$detail" = "-" ] && detail=""
    if [ "$detail" = "trusted=false" ]; then
      echo "$label: $settings (wired, NOT TRUSTED — start \`codex\` once and review the hook)"
    else
      echo "$label: $settings (wired)"
    fi
  done <<EOF
$out
EOF
}

deploy_hook() {
  local cort_bin="$1"
  if [ "$WITH_HOOK" -ne 1 ]; then
    info "hook: skipped (--no-hook)"
    return 0
  fi
  if [ ! -x "$cort_bin" ]; then
    info "hook: cort not executable at $cort_bin — skipping"
    return 0
  fi
  # One call. The binary owns the table -- which harnesses ship, which file each one reads, which
  # dialect it speaks, which subcommand each event runs -- and this script owns none of it.
  #
  # It used to own all four, restated on every one of six calls, and bash's copy was the one that
  # ran: `cort hook-install --status --format kimi` was answering about ~/.claude/settings.json for
  # who knows how long, because no shipped path ever exercised the binary's own defaults. Two
  # implementations of one rule, and the one without a caller is the one that rots.
  #
  # `--command-prefix` is the single thing only this script knows: the harness must run the *shim*
  # at $BIN_DIR/cort, not the real executable behind it, and `check_hook_at` verifies exactly that.
  local line harness ev outcome settings detail
  while IFS=$'\t' read -r harness ev outcome settings detail; do
    [ -n "$harness" ] || continue
    case "$outcome" in
      installed) info "hook ($harness/$ev): wired -> cort in $settings" ;;
      updated)   info "hook ($harness/$ev): updated in $settings" ;;
      already_present) info "hook ($harness/$ev): already wired in $settings" ;;
      error)     info "hook ($harness/$ev): NOT wired — $detail" ;;
      *)         info "hook ($harness/$ev): $outcome $detail" ;;
    esac
    # Codex gates a hook behind a one-time review and the trust it persists is bound to the exact
    # entry, so writing or rewriting one always leaves it inert until reviewed again. Nothing
    # downstream can tell a stale trust from a current one; the honest moment to say so is here,
    # where we are the ones who moved it.
    if [ "$harness" = "codex" ] && { [ "$outcome" = "installed" ] || [ "$outcome" = "updated" ]; }; then
      info "hook (codex): Codex runs it only after a one-time review — start \`codex\` and trust the hook"
    fi
    # The manifest records the path the binary resolved, rather than one this script worked out for
    # itself. That keeps it a record of what actually happened -- which is what uninstall needs --
    # instead of a second copy of the resolution rule.
    case "$harness" in
      claude-code) record_manifest "hook_settings" "$settings" ;;
      codex)       record_manifest "hook_settings_codex" "$settings" ;;
      kimi-code)   record_manifest "hook_settings_kimi" "$settings" ;;
    esac
  done <<EOF
$("$cort_bin" hook-install --all --lean --command-prefix "$cort_bin" 2>/dev/null)
EOF
}

remove_hook() {
  local cort_bin
  cort_bin="$(manifest_get cort_bin || true)"
  [ -n "$cort_bin" ] && [ -x "$cort_bin" ] || cort_bin="$(command -v cort 2>/dev/null || true)"
  if [ -z "$cort_bin" ] || [ ! -x "$cort_bin" ]; then
    info "hook: no cort to unwire with — leaving the settings files alone"
    return 0
  fi

  # The binary's own table first: same six entries the deploy wired, resolved the same way, so an
  # uninstall cannot miss a file the install created because this script disagreed about its path.
  local harness ev outcome settings detail command
  local seen=""
  while IFS=$'\t' read -r harness ev outcome settings detail command; do
    [ -n "$harness" ] || continue
    case "$seen" in *"|$settings|"*) continue ;; esac
    seen="$seen|$settings|"
    case "$outcome" in
      removed)     info "hook: unwired from $settings" ;;
      not_present) info "hook: nothing of ours in $settings" ;;
      error)       info "hook: could not unwire $settings — $detail" ;;
    esac
  done <<EOF
$("$cort_bin" hook-install --all --remove --lean 2>/dev/null)
EOF

  # Then anything the manifest records that the table did not reach. The manifest is the record of
  # what actually happened, so it is the only thing that knows where an entry went when a harness
  # home variable pointed somewhere else at install time than it does now. Reported by exit status
  # rather than by reading the reply, because a regex over our own JSON is the second parser this
  # refactor exists to delete.
  local key path
  for key in hook_settings hook_settings_codex hook_settings_kimi; do
    path="$(manifest_get "$key" || true)"
    [ -n "$path" ] || continue
    case "$seen" in *"|$path|"*) continue ;; esac
    local fmt_arg=""
    case "$key" in
      hook_settings_codex) fmt_arg="--format codex" ;;
      hook_settings_kimi)  fmt_arg="--format kimi" ;;
    esac
    if "$cort_bin" hook-install --settings "$path" $fmt_arg --remove >/dev/null 2>&1; then
      info "hook: unwired from $path (recorded in the manifest, outside the current table)"
    else
      info "hook: could not unwire $path"
    fi
  done
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
  local crate_bin="$SCRIPT_DIR/rust/target/release/cort"

  # Always ask cargo to build. Cargo owns freshness: an up-to-date tree costs a fraction of a
  # second, while "does the artifact exist?" does not — `git pull` leaves rust/target/ (ignored)
  # in place, so an existence check would happily ship the previous build's binary.
  command -v cargo >/dev/null 2>&1 || die "cort needs cargo (rustup) to build — rerun with --with-rustup"
  info "building cort (cargo build --release --locked)"
  ( cd "$SCRIPT_DIR/rust" && cargo build --release --locked ) || die "cargo build --release failed"
  [ -x "$crate_bin" ] || die "cort binary missing after build: $crate_bin"

  rm -rf "$CORT_HOME"
  mkdir -p "$CORT_HOME"
  # The binary locates its ast-grep pack via CORT_PACK_DIR: ship the pack next to it.
  cp "$crate_bin" "$CORT_HOME/cort"
  cp -R "$SCRIPT_DIR/src/pack" "$CORT_HOME/pack"
  chmod 755 "$CORT_HOME/cort"

  mkdir -p "$BIN_DIR"
  local shim="$BIN_DIR/cort"
  cat > "$shim.tmp" <<SHIM
#!/usr/bin/env bash
if [ "\$1" = "--version" ]; then echo "cort $CORT_VERSION (rust)"; exit 0; fi
CORT_PACK_DIR="$CORT_HOME/pack" exec "$CORT_HOME/cort" "\$@"
SHIM
  chmod 755 "$shim.tmp"
  mv "$shim.tmp" "$shim"
  record_manifest "cort_bin" "$shim"
  ( cd / && "$shim" status >/dev/null 2>&1 || true )
  info "installed cort $CORT_VERSION (rust) -> $shim"
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
  # Read-only: --status never writes. A skill deployed without the hook is half the routing, and
  # --check is the only place that can say so before the numbers go missing.
  # Ask the binary this installation owns, not whatever `cort` PATH resolves to. `deploy_hook` wires
  # an absolute path, so on a machine carrying two copies -- a newer one earlier in PATH -- the PATH
  # copy would answer for a hook that fires the other one, and --check would print OK about a binary
  # it never asked. That divergence is the thing this line exists to catch. The manifest is the
  # record of what was installed; resolve_bin_dir is the fallback before there is one.
  local managed_cort
  managed_cort="$(manifest_get cort_bin)"
  if [ -z "$managed_cort" ]; then
    resolve_bin_dir
    managed_cort="$BIN_DIR/cort"
  fi
  if [ -x "$managed_cort" ]; then
    # One query, six answers, and the same table the deploy used -- so --check cannot disagree with
    # the installer about which files exist, which is a way this pair has been wrong before. Every
    # entry is still reported on its own line: a file can carry one of ours and not the other, and a
    # single "wired" would hide it.
    check_all_hooks "$managed_cort"
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
  if [ -f "$CODEX_SKILL_DEST" ]; then
    if skill_is_managed "$CODEX_SKILL_DEST"; then
      echo "skill_codex: $CODEX_SKILL_DEST (managed)"
    else
      echo "skill_codex: $CODEX_SKILL_DEST (UNMANAGED)"
      ok=0
    fi
  else
    echo "skill_codex: $CODEX_SKILL_DEST (NOT INSTALLED)"
    ok=0
  fi
  if [ -f "$MANIFEST_FILE" ]; then
  # Indexes. An index built on an older commit still answers, and says `stale=true` when it does --
  # but the hook that used to be the only thing saying so was measured at 19 re-index runs against
  # 2,700+ fires, so it is named here too, where somebody is already reading. `cort projects` owns
  # the comparison (it has the head the row was built at and the head the tree is on now); this is
  # a report, not a decision, so a stale index is listed and never fails the check -- the refresh
  # hook closes the gap on its own from the next edit onward.
  local proj_json stale_names gone_names
  if proj_json="$("$managed_cort" projects 2>/dev/null)"; then
    stale_names="$(printf '%s' "$proj_json" | awk -F'"' '/"name":/{n=$4} /"stale": true/{print "    "n}')"
    # Keys come out in alphabetical order, so `exists` is read before the `name` it belongs to and
    # the flag has to be held until the object's last field. Keying off `name` alone named the
    # previous project.
    gone_names="$(printf '%s' "$proj_json" | awk -F'"' '/"exists": false/{e=1} /"exists": true/{e=0} /"name":/{n=$4} /"stale"/{if (e) print "    "n}')"
    if [ -n "$stale_names" ]; then
      echo "indexes: STALE — re-run \`cort index\` in each (or let the refresh hook catch up):"
      printf '%s\n' "$stale_names"
    else
      echo "indexes: all current"
    fi
    if [ -n "$gone_names" ]; then
      echo "  directories gone (candidates for \`cort delete\`):"
      printf '%s\n' "$gone_names"
    fi
  else
    echo "indexes: could not query — installed cort predates \`cort projects --stale\`"
  fi
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
  remove_hook
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

    skill_ag_codex="$(manifest_get skill_ast_grep_codex || true)"
    remove_managed_skill_at "${skill_ag_codex:-$CODEX_SKILL_DEST}" skill_ast_grep_codex

    if [ -n "$skill_ag" ] && [ -f "$skill_ag" ]; then
      if skill_is_managed "$skill_ag"; then
        rm -f "$skill_ag" "$(skill_stamp_for "$skill_ag")"
        info "removed $skill_ag"
        record_deploy "$skill_ag" "absent"
        rmdir "$(dirname "$skill_ag")" 2>/dev/null || true
      else
        info "skill_ast_grep no longer managed — skipping: $skill_ag"
      fi
    elif [ -f "$AST_GREP_SKILL_DEST" ] && skill_is_managed "$AST_GREP_SKILL_DEST"; then
      rm -f "$AST_GREP_SKILL_DEST" "$(skill_stamp_for "$AST_GREP_SKILL_DEST")"
      info "removed $AST_GREP_SKILL_DEST"
      record_deploy "$AST_GREP_SKILL_DEST" "absent"
      rmdir "$(dirname "$AST_GREP_SKILL_DEST")" 2>/dev/null || true
    else
      info "skill_ast_grep not managed — skipping"
    fi

    if [ -n "$skill_xg" ] && [ -f "$skill_xg" ]; then
      if skill_is_managed "$skill_xg"; then
        rm -f "$skill_xg" "$(skill_stamp_for "$skill_xg")"
        info "removed $skill_xg"
        record_deploy "$skill_xg" "absent"
        rmdir "$(dirname "$skill_xg")" 2>/dev/null || true
      else
        info "skill_xgrep no longer managed — skipping: $skill_xg"
      fi
    elif [ -f "$SKILL_DEST" ] && skill_is_managed "$SKILL_DEST"; then
      rm -f "$SKILL_DEST" "$(skill_stamp_for "$SKILL_DEST")"
      info "removed $SKILL_DEST"
      record_deploy "$SKILL_DEST" "absent"
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
      rm -f "$SKILL_DEST" "$(skill_stamp_for "$SKILL_DEST")"
      info "removed $SKILL_DEST (managed)"
      record_deploy "$SKILL_DEST" "absent"
      rmdir "$(dirname "$SKILL_DEST")" 2>/dev/null || true
    else
      info "no manifest and skill not managed — nothing to remove for skill"
    fi
    if [ -f "$AST_GREP_SKILL_DEST" ] && skill_is_managed "$AST_GREP_SKILL_DEST"; then
      rm -f "$AST_GREP_SKILL_DEST" "$(skill_stamp_for "$AST_GREP_SKILL_DEST")"
      info "removed $AST_GREP_SKILL_DEST (managed)"
      record_deploy "$AST_GREP_SKILL_DEST" "absent"
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

  # Preflight every destination (claude + codex skill, optional xgrep) before any mutation
  preflight_skill_at "$SCRIPT_DIR/$AST_GREP_SKILL_SRC_REL" "$AST_GREP_SKILL_DEST"
  preflight_skill_at "$SCRIPT_DIR/$AST_GREP_SKILL_SRC_REL" "$CODEX_SKILL_DEST"
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
  deploy_skill_at "$SCRIPT_DIR/$AST_GREP_SKILL_SRC_REL" "$CODEX_SKILL_DEST" "skill_ast_grep_codex"
  if [ "$WITH_XGREP" -eq 1 ]; then
    deploy_skill_at "$SCRIPT_DIR/$SKILL_SRC_REL" "$SKILL_DEST" "skill_xgrep"
  fi
  deploy_hook "$BIN_DIR/cort"
  ensure_path_block
  record_manifest "manifest_version" "2"

  echo ""
  echo "Done. Verify with: cort --version && ast-grep --version && cat $AST_GREP_SKILL_DEST | head -5"
  echo "Skill also deployed for Codex: $CODEX_SKILL_DEST"
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
