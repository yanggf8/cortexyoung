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
pub const AST_GREP_PINNED: &str = "0.45.2";

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
                "67aff72dd2994bf152fcc3a8a09cf93b13193abe59f39393095167c729af2015",
            ),
            (
                "app-aarch64-unknown-linux-gnu.zip",
                "e67ee2f5928b4d77a472114edf6e227d90fefe22fa47e7a78db187c55d206564",
            ),
            (
                "app-x86_64-apple-darwin.zip",
                "037e5b4a9aed2ba03a2b4710e4fe3439d5d1154d1266d5e8f9f6df7452169181",
            ),
            (
                "app-aarch64-apple-darwin.zip",
                "1fc21214234bf6f5a3f841d5b2493a4fc4b6087f69b055c9ad5f94f77c0ab76e",
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
