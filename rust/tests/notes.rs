use cort::notes::{
    decode_project_dir, frontmatter, looks_secret, project_for, scan, Decoded, ScanStatus,
};
use std::fs;
use std::path::Path;
use std::process::Command;

fn mkdir(path: &Path) {
    fs::create_dir_all(path).unwrap();
}

#[test]
fn scan_is_bounded_and_skips_non_files() {
    let temp = tempfile::tempdir().unwrap();
    let memory = temp.path().join("projects/-demo/memory");
    mkdir(&memory.join("nested"));
    fs::write(memory.join("a.md"), b"hello").unwrap();
    fs::write(memory.join("big.md"), vec![b'x'; 65537]).unwrap();
    fs::write(memory.join("ignored.txt"), b"no").unwrap();
    fs::write(memory.join("nested/hidden.md"), b"no").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(memory.join("a.md"), memory.join("linked.md")).unwrap();
    let rows = scan(temp.path());
    assert!(rows.complete);
    let rows = rows.notes;
    #[cfg(unix)]
    assert_eq!(rows.len(), 3);
    #[cfg(not(unix))]
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].encoded_dir_name, "-demo");
    assert_eq!(rows[0].file_name, "a.md");
    assert_eq!(rows[0].bytes.as_deref(), Some(b"hello".as_slice()));
    assert_eq!(
        rows[0].sha256.as_deref(),
        Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );
    assert_eq!(rows[1].status, ScanStatus::SkippedSize);
    assert!(rows[1].bytes.is_none());
    #[cfg(unix)]
    assert_eq!(rows[2].status, ScanStatus::SkippedSymlink);
}

#[test]
fn scan_reports_missing_and_undecodable_inputs() {
    let temp = tempfile::tempdir().unwrap();
    assert!(scan(temp.path()).complete);
    assert!(scan(temp.path()).notes.is_empty());
    let memory = temp.path().join("projects/-demo/memory");
    mkdir(&memory);
    fs::write(memory.join("bad.md"), [0xff]).unwrap();
    // APFS refuses a name that is not UTF-8 (EILSEQ), so only Linux can make one.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        let name = std::ffi::OsString::from_vec(b"bad\xff.md".to_vec());
        fs::write(memory.join(name), b"valid").unwrap();
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&memory, temp.path().join("projects/-linked")).unwrap();
        std::os::unix::fs::symlink(&memory, temp.path().join("projects/-other/memory")).ok();
    }
    let report = scan(temp.path());
    assert!(report.complete);
    assert!(report.notes.iter().any(|note| note.file_name == "bad.md"
        && note.status == ScanStatus::SkippedEncoding
        && note.bytes.is_none()));
    #[cfg(target_os = "linux")]
    assert!(report
        .notes
        .iter()
        .any(|note| note.status == ScanStatus::SkippedName));
    #[cfg(unix)]
    {
        assert!(report
            .notes
            .iter()
            .any(|note| note.status == ScanStatus::SkippedSymlink));
    }
}

#[cfg(unix)]
#[test]
fn decode_refuses_matching_symlink() {
    let temp = tempfile::tempdir().unwrap();
    mkdir(&temp.path().join("actual"));
    std::os::unix::fs::symlink(temp.path().join("actual"), temp.path().join("match")).unwrap();
    assert_eq!(decode_project_dir("-match", temp.path()), Decoded::Unknown);
}

#[test]
fn decode_requires_one_existing_directory() {
    let temp = tempfile::tempdir().unwrap();
    mkdir(&temp.path().join("home/a/b-c"));
    assert_eq!(
        decode_project_dir("-home-a-b-c", temp.path()),
        Decoded::Unique(temp.path().join("home/a/b-c"))
    );
    mkdir(&temp.path().join("home/a-b/c"));
    assert!(
        matches!(decode_project_dir("-home-a-b-c", temp.path()), Decoded::Ambiguous(paths) if paths.len() == 2)
    );
    assert_eq!(
        decode_project_dir("-missing", temp.path()),
        Decoded::NotFound
    );
    mkdir(&temp.path().join("home/yanggf/.claude-code-router"));
    assert_eq!(
        decode_project_dir("-home-yanggf--claude-code-router", temp.path()),
        Decoded::Unique(temp.path().join("home/yanggf/.claude-code-router"))
    );
}

#[test]
fn decode_caps_search_without_guessing() {
    let temp = tempfile::tempdir().unwrap();
    let punctuation = ['!', '@', '#', '$', '%', '^', '&', '(', ')', '_'];
    for first in punctuation {
        for second in punctuation {
            mkdir(&temp.path().join(format!("{first}{second}")));
        }
    }
    // More than 64 root candidates encode as "---"; none may be guessed.
    assert!(matches!(
        decode_project_dir("----a", temp.path()),
        Decoded::Unknown
    ));
}

#[test]
fn project_uses_origin_even_with_override() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    mkdir(&repo);
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "https://example.test/team/repo.git"
        ])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let expected = project_for(&repo).unwrap();
    let prior = std::env::var_os("CORT_MEMORY_PROJECT");
    std::env::set_var("CORT_MEMORY_PROJECT", "wrong-project");
    let actual = project_for(&repo);
    match prior {
        Some(value) => std::env::set_var("CORT_MEMORY_PROJECT", value),
        None => std::env::remove_var("CORT_MEMORY_PROJECT"),
    }
    assert_eq!(actual.unwrap(), expected);
}

#[test]
fn detects_secret_families() {
    let cases = [
        (
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature0123456789",
            "jwt",
        ),
        (concat!("ghp_", "1234567890ABCDEF"), "github_token"),
        (concat!("gho_", "1234567890ABCDEF"), "github_token"),
        (concat!("ghs_", "1234567890ABCDEF"), "github_token"),
        (concat!("github_pat_", "1234567890ABCDEF"), "github_token"),
        (concat!("sk-", "abcdefghijklmnopqrstuvwxyz"), "api_token"),
        (
            concat!("sk_live_", "abcdefghijklmnopqrstuvwxyz"),
            "api_token",
        ),
        (concat!("xoxb-", "1234567890ABCDEF"), "slack_token"),
        (concat!("xoxa-", "1234567890ABCDEF"), "slack_token"),
        (concat!("xoxp-", "1234567890ABCDEF"), "slack_token"),
        (concat!("xoxr-", "1234567890ABCDEF"), "slack_token"),
        (concat!("AKIA", "1234567890123456"), "aws_access_key"),
        (concat!("ASIA", "1234567890123456"), "aws_access_key"),
        ("-----BEGIN PRIVATE KEY-----", "pem_key"),
        ("postgres://user:pass@db.example/test", "url_userinfo"),
        ("Authorization: Basic Abc123XYZ", "authorization"),
        ("TOKEN=abc123XYZ", "credential_assignment"),
        ("password: abc123XYZ", "credential_assignment"),
        ("api_key: abc123XYZ", "credential_assignment"),
        ("private_key=abc123XYZ", "credential_assignment"),
        (
            "libsql://db.example?authToken=abc123XYZ",
            "url_query_secret",
        ),
        (
            "curl -H \"Authorization: Bearer Abc123XYZ\" https://example.test",
            "authorization",
        ),
        (
            "curl -u user:Abc123XYZ https://example.test",
            "curl_userinfo",
        ),
        ("curl -H 'X-Api-Key: Abc123XYZ'", "authorization"),
        ("Cookie: session=Abc123XYZ", "authorization"),
        (
            concat!(
                "DATABASE_URL=postgres://user:Abc123XYZ@db\nSTRIPE_SECRET_KEY=sk_",
                "live_1234567890ABCDEF"
            ),
            "api_token",
        ),
        (concat!("FlyV1 fm1_", "1234567890ABCDEF"), "api_token"),
        ("1//0abcDEFghi123JKLmnop456QRS", "high_entropy"),
        ("`Abc123Def456Ghi789Jkl012Mno3`", "high_entropy"),
        ("TOKEN=abc123XYZ # rotate", "credential_assignment"),
        (
            "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz+/",
            "high_entropy",
        ),
        (
            "AaBbCcDdEeFfGgHhIiJjKkLl0123456789AaBbCcDdEeFfGgHhIiJjKkLl",
            "high_entropy",
        ),
    ];
    for (body, rule) in cases {
        assert_eq!(looks_secret(body), Some(rule), "{body}");
    }
}

#[test]
fn ordinary_notes_and_placeholders_are_allowed() {
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let digest = "0123456789abcdef".repeat(4);
    let body = format!("Commit {sha}\nsha256 {digest}\nSee /home/person/work/projects/long-name/docs/example-file.md\nURL https://example.test/path\nthe group token covers it\ntoken=<...>\nsecret=...\npassword=xxx\napi_key=${{API_KEY}}\n");
    assert_eq!(looks_secret(&body), None);
    assert_eq!(looks_secret("Authorization: Bearer <TOKEN>"), None);
    for line in [
        "password: required",
        "token: group",
        "tokenizer: gpt-4o",
        "Tokens: 4096",
        "enable Bearer authentication",
        "Authorization: Bearer",
    ] {
        assert_eq!(looks_secret(line), None, "{line}");
    }
}

#[test]
fn frontmatter_reads_and_sanitizes_scalars() {
    let body = "---\nname: one\t two\ndescription: héllo\t world\u{0007} again\nmetadata:\n  type: project\n---\nbody";
    let found = frontmatter(body);
    assert_eq!(found.name.as_deref(), Some("one two"));
    assert_eq!(found.description.as_deref(), Some("héllo world again"));
    assert_eq!(found.kind.as_deref(), Some("project"));
    assert_eq!(frontmatter("plain text").name, None);
    assert_eq!(frontmatter("---\nname: no closing marker").name, None);
    let long = format!("---\ndescription: {}\n---", "é".repeat(210));
    assert_eq!(frontmatter(&long).description.unwrap().chars().count(), 200);
    let extra = frontmatter("---\nname : foo # comment\ndescription: >\n  first line\n  second line\nmetadata:\n  type: project # comment\n---");
    assert_eq!(extra.name.as_deref(), Some("foo"));
    assert_eq!(extra.description.as_deref(), Some("first line second line"));
    assert_eq!(extra.kind.as_deref(), Some("project"));
    assert_eq!(frontmatter("---\nname:\n---").name, None);
    assert_eq!(
        frontmatter("---\nname: a\u{200b}b\n---").name.as_deref(),
        Some("a b")
    );
}

/// Lines shaped like the memory notes this repo's agents actually write: slugs, dates, commit
/// ids, sha256 digests, paths, and prose that uses the word "token". None of these is a secret,
/// and a screen that rejected them would drop most notes.
#[test]
fn realistic_note_prose_is_not_a_secret() {
    for line in [
        "- [10/28 隔日量測檢查](measure-next-day-check-2026-10-28.md) — 三台 32e1162e(watchdog)部署後",
        "- **Two token systems coexist** — `gwebcdb-mint` issues one GROUP token good for every DB.",
        "harnessuse 是 turso:// 型，armo group token 涵蓋但 per-DB resolve 卡 libsql:// 硬檢查",
        "The id is 9c7d6368a1455be69360f3e370c424527a5903dc0bf5c2ef14ceb7510b0ba968 (sha256).",
        "Commit 774dbfa7c0ffee1234567890abcdef1234567890 wired hook-recall into SessionStart.",
        "See [[use-cort-memory-for-project-decisions]] and [[one-build-at-a-time-and-resource-checks]].",
        "Token budget: 6000 bytes; the password prompt is handled by armo, not by cort.",
        "path: /home/yanggf/.local/share/cortexyoung/memory/turso-memory/809d782c461a205f/memory.db",
    ] {
        assert_eq!(looks_secret(line), None, "{line}");
    }
    assert_eq!(
        looks_secret("TURSO_AUTH_TOKEN=s3cr3tValue42"),
        Some("credential_assignment")
    );
    assert_eq!(
        looks_secret("key Zx9Qm2Lp8Rt4Vw7Ny1Bc6Hd3Jf5Kg0Ms"),
        Some("high_entropy")
    );
}

/// Shapes found in the real notes on this machine on 2026-10-27: 587 of 680 notes were
/// dropped for `originSessionId` (a "session" component) and 17 for an `sk-` inside a slug.
#[test]
fn claude_code_note_metadata_is_not_a_secret() {
    for line in [
        "  originSessionId: 753a369a-8270-4327-a497-9a5043bcd659",
        "name: ask-user-question-pitfalls",
        "- [Ask am I over-correcting](ask-am-i-over-correcting.md) — 抓到真缺陷後",
        "診斷時看 `auth.rs::email_delivery_configured`。",
        "`credentials::resolve_cached_or_mint`. With `TURSO_*_TOKEN` set,",
        "helper: `credential.https://github.com.helper \"\"`（reset）",
        "B0＋Muse 七項修（token=sid、permission=JsonNode）",
        "只有這兩處（CiInputFilter／CwC2bBasicAuthFilter／LocalGatewayFilter）",
        "export PERSONA_REGISTRY_DB_TOKEN=\"$TURSO_WRITE_TOKEN\"",
        "stored as `export CF_ACCESS_TOKEN='cfat_...'`",
    ] {
        assert_eq!(looks_secret(line), None, "{line}");
    }
    assert_eq!(
        looks_secret("Authorization: dGhpc2lzQVRva2VuMTIz"),
        Some("authorization")
    );
    assert_eq!(
        looks_secret("SESSION_KEY=f00dCafe12345678"),
        Some("credential_assignment")
    );
    assert_eq!(
        looks_secret(concat!("token is sk-", "ant-api03-AbCdEfGh12345678")),
        Some("api_token")
    );
}

#[test]
fn scan_lists_only_directories_it_read_to_the_end() {
    let temp = tempfile::tempdir().unwrap();
    let projects = temp.path().join("projects");
    mkdir(&projects.join("-emptied/memory"));
    mkdir(&projects.join("-notes/memory/dir.md"));
    fs::write(projects.join("-notes/memory/a.md"), b"a").unwrap();
    mkdir(&projects.join("-nomemory"));
    #[cfg(unix)]
    {
        mkdir(&temp.path().join("elsewhere"));
        mkdir(&projects.join("-linkedmem"));
        std::os::unix::fs::symlink(
            temp.path().join("elsewhere"),
            projects.join("-linkedmem/memory"),
        )
        .unwrap();
    }
    let report = scan(temp.path());
    assert!(report.complete);
    let listed: Vec<_> = report.listed_dirs.iter().map(String::as_str).collect();
    assert_eq!(listed, ["-emptied", "-notes"]);
    assert!(report
        .notes
        .iter()
        .any(|n| n.file_name == "dir.md" && n.status == ScanStatus::SkippedNotFile));
}

/// Muse's round-2 false negatives (2026-10-27): each of these holds a usable credential.
#[test]
fn round_two_credential_shapes_are_caught() {
    for line in [
        "SESSION_ID=Zq81kLmN02pQrStUv",
        "https://acct.blob.core.windows.net/c/f?sv=2022&sig=Ab12Cd34Ef56%3D",
        "redis://:p4ssWord99@cache.internal:6379",
        "https://ghx1Tok3nValue9z@git.example.com/repo.git",
        "password: $2b$12$KIXQ8wJ1y0T5u7Zp3cM2a.",
        concat!("aws secret wJalrXUtnFEMI/K7MDENG", "/bPxRfiCYzEXAMPLEKEY99"),
    ] {
        assert!(looks_secret(line).is_some(), "{line}");
    }
    for line in [
        "  originSessionId: 753a369a-8270-4327-a497-9a5043bcd659",
        "ssh://git@github.com/yanggf8/claudecat",
        "TOKEN=<token>",
        "export TOKEN=${TURSO_TOKEN}",
    ] {
        assert_eq!(looks_secret(line), None, "{line}");
    }
}

#[test]
fn colon_assignments_need_a_key_position() {
    for line in [
        "Sibling fix from the same session: [[out_of_pool_citation_gate]].",
        "user judged the whole session: \"all-defense\" for now",
    ] {
        assert_eq!(looks_secret(line), None, "{line}");
    }
    for line in [
        "{\"password\": \"Hunter2isMyPass\"}",
        "- api_key: Zq81kLmN02pQ",
        "curl -H 'X-Api-Key: Zq81kLmN02pQ' https://api.example.com",
    ] {
        assert!(looks_secret(line).is_some(), "{line}");
    }
}

/// Muse's round-3 findings (2026-10-27).
#[test]
fn round_three_credential_shapes_are_caught() {
    for line in [
        "session_id: 550e8400-e29b-41d4-a716-446655440000",
        "password_hash=$argon2id$v=19$m=65536,t=3,p=4$c2FsdHNhbHQ$Zm9vYmFyYmF6",
        "curl -H X-Api-Key: Zq81kLmN02pQ https://api.example.com",
        "user=admin; password: Hunter2isMyPass",
    ] {
        assert!(looks_secret(line).is_some(), "{line}");
    }
    assert_eq!(looks_secret("export TOKEN=$TURSO_TOKEN"), None);
}
