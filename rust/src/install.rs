//! Install facts: the single home for what a release is.
//!
//! Three things used to live in `install.sh` in bash, restated from sources Rust already owned:
//! the shim template, the ast-grep provenance, and the manifest key-set. Bash's copies were the
//! ones that shipped — `HOOK_TARGETS` already taught this repository what happens then
//! (`docs/2026-09-03-installer-dedup-and-attribution.md` §2, §3). Nothing in this module decides
//! anything at install time; it only states facts. `install.sh` queries them from the just-built
//! binary and writes files; that is all bash is for here.

/// Everything needed to decide *which* ast-grep to fetch: the version pin, the release repo,
/// the crate name for the cargo fallback, and the per-asset checksums for fail-closed
/// verification. The pin used to live in `ast_grep.rs` while the version, repo and checksums lived
/// in bash — two homes equal only by maintenance. There is one now.
pub const AST_GREP_PINNED: &str = "0.45.3";

pub struct AstGrepProvenance {
    pub version: &'static str,
    pub repo: &'static str,
    pub crate_name: &'static str,
    pub assets: &'static [(&'static str, &'static str)],
}

/// The one home for ast-grep provenance.
pub fn ast_grep_provenance() -> AstGrepProvenance {
    AstGrepProvenance {
        version: AST_GREP_PINNED,
        repo: "ast-grep/ast-grep",
        crate_name: "ast-grep",
        assets: &[
            (
                "app-x86_64-unknown-linux-gnu.zip",
                "f8ac830881339d1edee6b2652f54798c0f4da5a827f2db38a08ee31117783ce8",
            ),
            (
                "app-aarch64-unknown-linux-gnu.zip",
                "b39cfbc58da4b869a88b8a4bc57bd5deb0d24541e704cf7c257da7b53ec81c8f",
            ),
            (
                "app-x86_64-apple-darwin.zip",
                "b2ffd26f42810340326a9e8a084bdc3647a8795c1a3f21fc06bd7bef3c7c5b2c",
            ),
            (
                "app-aarch64-apple-darwin.zip",
                "6d2279dea5bea2ad79c66ea93f5fe54ba926e398a8a26de76c56db68fe59eac6",
            ),
        ],
    }
}

impl AstGrepProvenance {
    pub fn checksum_for(&self, asset: &str) -> Option<&'static str> {
        self.assets
            .iter()
            .find(|(a, _)| *a == asset)
            .map(|(_, s)| *s)
    }
}

/// Every key a fresh install may write to the manifest. This is the authoritative set: install.sh
/// must name a key to write it, so the literals stay in the script, but nothing may exist here
/// that is not named below, and the test above enforces it. A key added to the script without
/// being added here fails the build — not uninstall, not in upgrade, where it would surface as a
/// leaked artifact.
// `profile` is written by the PATH-block step: which rc file carries the BIN_DIR export.
// Uninstall re-scans the candidate files rather than reading it, but the installer has always
// written it — and until it went through `record_manifest`, this set never saw it and every
// upgraded manifest read `keys no release knows: profile` (found deploying).
pub const MANIFEST_KEYS: &[&str] = &[
    "manifest_version",
    "cort_bin",
    "ast_grep_bin",
    // No release writes these two any more -- the xgrep skill and the `xg` binary were retired on
    // 2026-09-10 -- but they stay known, because this list is what tells an upgrade whether a
    // manifest holds a key no release recognises. Drop them and every machine that once took
    // `--with-xgrep` starts reading `keys no release knows`, on the very manifest entries
    // `--uninstall` still needs to find the leftovers it must remove.
    "legacy_xg_bin",
    "skill_xgrep",
    "skill_ast_grep",
    "skill_ast_grep_codex",
    "hook_settings",
    "hook_settings_codex",
    "hook_settings_kimi",
    "profile",
];

/// Keys no fresh install writes but old manifests may hold, renamed by `migrate_manifest_v2`.
/// Readable, never written. Uninstall must still honour them, which is why they are named rather
/// than forgotten.
pub const MANIFEST_LEGACY_KEYS: &[&str] = &["xg_bin", "skill"];

/// Render the `$BIN_DIR/cort` shim for the given `CORT_HOME`.
///
/// Two lines, and the shim now forwards *everything*. It used to intercept `--version` and echo a
/// string baked in at install time, returning before the `exec` — so the one command `--check`
/// parses was the one command that never reached the payload, and it answered the same whether the
/// generation behind it was current, stale, or missing. The design spec had already written that
/// check off as one that "can never be inconsistent"
/// (`docs/superpowers/specs/2026-09-06-cort-upgrade-design.md` §69, §267); `main.rs::wants_version`
/// now answers from the binary, so the same `--check` line names the payload that would really run.
///
/// The absolute paths still resolve at exec time rather than install time — that late binding is
/// what makes the generation flip (`install.sh`, Task 3a) take effect for already-installed shims,
/// and with the intercept gone `--version` finally travels through it like every other verb.
pub fn render_shim(cort_home: &str) -> String {
    format!("#!/usr/bin/env bash\nCORT_PACK_DIR=\"{cort_home}/pack\" exec \"{cort_home}/cort\" \"$@\"\n")
}
