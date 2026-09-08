//! Install facts have one home: this crate. install.sh holds no copy of any of them, and these
//! tests are what makes that claim enforceable rather than aspirational.

use cort::install::render_shim;

/// The shim is three lines and every one of them is load-bearing: the `--version` intercept keeps
/// `--check` from executing the binary, and the absolute paths are resolved at exec time, which is
/// what makes the generation flip take effect for already-installed shims.
#[test]
fn the_shim_has_exactly_the_shape_the_installer_ships() {
    let version = env!("CARGO_PKG_VERSION");
    let expected = format!(
        "#!/usr/bin/env bash\nif [ \"$1\" = \"--version\" ]; then echo \"cort {version} (rust)\"; exit 0; fi\nCORT_PACK_DIR=\"{dir}/pack\" exec \"{dir}/cort\" \"$@\"\n",
        dir = "/home/someone/.local/share/cortexyoung/cort"
    );
    let shim = render_shim("/home/someone/.local/share/cortexyoung/cort");
    assert_eq!(shim, expected);
}

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
    // while shipping a lie.
    for (asset, sha) in [
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
/// comments. Every literal the old code held is a needle here — checking one hash while three
/// remain is a test that passes around the defect. The crate name (`ast-grep`) is deliberately
/// absent: it is also the binary name and appears legitimately throughout the script.
#[test]
fn install_sh_names_no_ast_grep_version_repo_or_checksum() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    for line in installer.lines() {
        let code = line.split('#').next().unwrap_or("");
        for needle in [
            "0.45.2",
            "ast-grep/ast-grep",
            "67aff72dd2994bf152fcc3a8a09cf93b13193abe59f39393095167c729af2015",
            "e67ee2f5928b4d77a472114edf6e227d90fefe22fa47e7a78db187c55d206564",
            "037e5b4a9aed2ba03a2b4710e4fe3439d5d1154d1266d5e8f9f6df7452169181",
            "1fc21214234bf6f5a3f841d5b2493a4fc4b6087f69b055c9ad5f94f77c0ab76e",
        ] {
            assert!(
                !code.contains(needle),
                "install.sh names {needle} in code: {line}"
            );
        }
    }
}
/// test in this file while the old heredoc keeps shipping — the home exists but nobody lives in
/// it. This asserts the call, not the shape.
#[test]
fn install_sh_renders_the_shim_through_the_verb() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    assert!(
        installer.contains("internal-shim --cort-home"),
        "install.sh must query the just-built binary instead of carrying its own heredoc"
    );
    assert!(
        !installer.contains("exec \"$CORT_HOME/cort\""),
        "the heredoc template must be gone from install.sh"
    );
}
/// equal the crate version, or --check reports MISMATCH on a correct install. The shim itself no
/// longer depends on it; this test is the only thing keeping the two in step.
#[test]
fn install_sh_version_matches_the_crate_version() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    let line = installer
        .lines()
        .find(|l| l.starts_with("CORT_VERSION="))
        .expect("install.sh sets CORT_VERSION");
    let shell_version = line.trim_start_matches("CORT_VERSION=").trim_matches('"');
    assert_eq!(shell_version, env!("CARGO_PKG_VERSION"));
}

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
                if !key.starts_with('$') && !cort::install::MANIFEST_KEYS.contains(&key) {
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
            if args.len() >= 6
                && !args[5].starts_with('$')
                && !cort::install::MANIFEST_KEYS.contains(&args[5])
            {
                unknown.push(args[5].to_string());
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
        // A trailing `(` means the function definition itself (`manifest_get() {`), not a call.
        let mut rest = code;
        while let Some(start) = rest.find("manifest_get") {
            rest = &rest[start + "manifest_get".len()..];
            if rest.trim_start_matches([' ', '\t']).starts_with('(') {
                rest = &rest[rest.find('(').unwrap_or(0) + 1..];
                continue;
            }
            let arg = rest.trim_start_matches([' ', '\t']);
            let key = if let Some(q) = arg.strip_prefix('"') {
                q.split('"').next().unwrap_or("")
            } else {
                arg.split([' ', '\t', ')', ';']).next().unwrap_or("")
            };
            if !key.is_empty()
                && !key.starts_with('$')
                && !cort::install::MANIFEST_KEYS.contains(&key)
                && !cort::install::MANIFEST_LEGACY_KEYS.contains(&key)
            {
                unknown.push(key.to_string());
            }
            if arg.len() < 2 {
                break;
            }
            rest = &arg[1..];
        }
        // Shape 2: the loop feeding manifest_get "$key" — every word after `in` is a key.
        if let Some(in_pos) = code.find("for key in ") {
            for word in code[in_pos + "for key in ".len()..]
                .split([' ', '\t', ';'])
                .map(str::trim)
                .filter(|w| !w.is_empty() && !w.starts_with('$') && *w != "do")
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

/// The only write shape is `record_manifest` — it stages the whole next manifest and swaps it
/// in with one rename, and it is what the key-membership parser above can see. The guard
/// therefore forbids ANY redirect targeting `$MANIFEST_FILE` itself (append or truncate,
/// echo or printf or cat — the printf shape is why "contains echo" was too narrow), while
/// reads that redirect OUT of the file (`grep ... > "$tmp"`) and the staged `mv` are the
/// sanctioned shapes. A raw append leaves the key half-written on interruption, and the
/// parser cannot enforce its key; a raw truncate write can lose the whole ledger. The echo
/// blind spot is how `profile` drifted out of MANIFEST_KEYS for its whole life (found by the
/// Codex review round), and the truncate shape — `cat "$tmp" > "$MANIFEST_FILE"` in
/// migrate_manifest_v2 — predates that and escaped this test's first draft (Kimi review
/// round).
#[test]
fn manifest_writes_go_through_record_manifest_only() {
    let installer = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../install.sh"));
    let mut raw = Vec::new();
    for (n, line) in installer.lines().enumerate() {
        let code = line.split('#').next().unwrap_or("");
        // A redirect whose target is the live manifest: `>` or `>>`, optional whitespace,
        // optional quote, then `$MANIFEST_FILE`. `grep -v x "$MANIFEST_FILE" > "$tmp"` only
        // READS the file — its redirect target is "$tmp" — so it is not matched; the staged
        // `mv -f "$tmp" "$MANIFEST_FILE"` is no redirect at all.
        let mut from = 0;
        while let Some(gt) = code[from..].find('>') {
            let after = code[from + gt + 1..].trim_start();
            let after = after.strip_prefix('"').unwrap_or(after);
            if after.starts_with("$MANIFEST_FILE") {
                raw.push(format!("install.sh:{}: {}", n + 1, line.trim()));
                break;
            }
            from += gt + 1;
        }
    }
    assert!(
        raw.is_empty(),
        "install.sh writes the manifest outside record_manifest: {raw:?}"
    );
}
