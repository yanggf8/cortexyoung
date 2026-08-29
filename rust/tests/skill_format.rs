//! Gate: every `skills/<name>/SKILL.md` in this repo is shaped the way the agent loaders parse it.
//!
//! Why this file exists at all: the deployed skill was invisible to Codex once (F-16, a marker
//! line written above the YAML fence) and shaped wrongly a second time (F-19, the same marker
//! moved *inside* the fence, where it is legal YAML but is not a documented key). Both times the
//! tests that ran were "is the file there" and "does it contain the marker" — nothing ever parsed
//! the document the way the consumer does, so a full green smoke suite kept certifying an artifact
//! no agent would route to. This test is that parser, run against the source of truth on every
//! `cargo test`.
//!
//! The rules below are the union of what the two loaders enforce and what the documented shape
//! allows. Measured behaviour behind them (see docs/2026-08-29 audit, F-16 and F-19):
//!   * Codex skips a skill whose fence does not open at byte 0, with no entry in the prompt.
//!   * Claude Code anchors on the same fence and, on no match, drops every field silently.
//!   * The frontmatter key set is closed: `name`, `description`, `license`, `allowed-tools`,
//!     `metadata`. A comment parses, but it is not part of the shape, and an installer that puts
//!     bookkeeping in the document has to delete it again before comparing bytes.

use std::fs;
use std::path::{Path, PathBuf};

const ALLOWED_KEYS: &[&str] = &[
    "name",
    "description",
    "license",
    "allowed-tools",
    "metadata",
];
const MAX_NAME_LEN: usize = 64;
const MAX_DESCRIPTION_LEN: usize = 1024;
// The string install.sh writes into its ownership stamp. It must never appear in a document it
// deploys; F-19 is exactly this string migrating from above the fence, into the fence, and out of
// the file altogether.
const INSTALLER_BOOKKEEPING: &str = "managed by cortexyoung install.sh";

fn skills_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rust/ sits next to skills/")
        .join("skills")
}

/// A parsed SKILL.md: the fence lines, the key/value pairs inside them, and the body after.
struct SkillDoc {
    keys: Vec<(String, String)>,
    body: String,
}

/// Parse exactly as little as the loaders do. Anything outside the documented shape is a hard
/// failure with the reason, because a silently-skipped skill is the failure mode being prevented.
fn parse(path: &Path) -> Result<SkillDoc, String> {
    let raw = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Err("starts with a UTF-8 BOM, so the fence is not at byte 0".to_string());
    }
    let text = String::from_utf8(raw).map_err(|e| format!("not valid UTF-8: {e}"))?;
    let mut lines = text.lines();
    let first = lines.next().ok_or("file is empty")?;
    if first != "---" {
        return Err(format!(
            "line 1 is {first:?}, expected the opening frontmatter fence '---'"
        ));
    }
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line == "---" {
            closed = true;
            break;
        }
        if line.is_empty() {
            return Err("blank line inside the frontmatter".to_string());
        }
        if line.starts_with('#') {
            return Err(format!(
                "{line:?} is a comment. YAML accepts it and no loader complains, which is how a                  marker line survived two releases: the block is for keys, and this repository's                  keys are documented and closed."
            ));
        }
        if line.starts_with(char::is_whitespace) {
            // A nested value, e.g. `metadata:` / `  short-description: ...`. Legal, and the key it
            // belongs to was already validated; install.sh's gate allows the same shape.
            if keys.is_empty() {
                return Err(format!(
                    "{line:?} continues a frontmatter key that never opened"
                ));
            }
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("{line:?} inside the frontmatter has no ':' separator"))?;
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("{line:?} has an unusable frontmatter key"));
        }
        if keys.iter().any(|(k, _)| k == key) {
            return Err(format!("duplicate frontmatter key {key:?}"));
        }
        keys.push((key.to_string(), value.trim().to_string()));
    }
    if !closed {
        return Err("frontmatter fence is never closed".to_string());
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    Ok(SkillDoc { keys, body })
}

fn skill_paths() -> Vec<PathBuf> {
    let root = skills_root();
    let mut out: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("cannot read skills root {}: {e}", root.display()))
        .filter_map(|entry| {
            let dir = entry.ok()?.path();
            let skill = dir.join("SKILL.md");
            dir.is_dir()
                .then(|| skill.exists().then_some(skill))
                .flatten()
        })
        .collect();
    out.sort();
    out
}

fn dir_name_of(skill: &Path) -> String {
    skill
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

#[test]
fn the_skills_are_not_vacuously_present() {
    let paths = skill_paths();
    assert!(
        paths.len() >= 2,
        "expected the ast-grep and xgrep skills, found {:?}",
        paths
    );
}

#[test]
fn every_skill_parses_with_a_fence_at_byte_zero() {
    for path in skill_paths() {
        parse(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}

#[test]
fn every_skill_uses_only_documented_keys() {
    for path in skill_paths() {
        let doc = parse_or_panic(&path);
        check_documented_keys(&path, &doc);
    }
}

#[test]
fn every_skill_names_its_own_directory() {
    for path in skill_paths() {
        let doc = parse_or_panic(&path);
        check_name_matches_directory(&path, &doc);
    }
}

#[test]
fn every_skill_describes_when_to_use_it() {
    for path in skill_paths() {
        let doc = parse_or_panic(&path);
        check_description(&path, &doc);
    }
}

#[test]
fn every_skill_has_a_body_after_the_fence() {
    for path in skill_paths() {
        let doc = parse_or_panic(&path);
        check_has_body(&path, &doc);
    }
}

#[test]
fn no_skill_carries_installer_bookkeeping() {
    for path in skill_paths() {
        check_no_bookkeeping(&path);
    }
}

fn parse_or_panic(path: &Path) -> SkillDoc {
    match parse(path) {
        Ok(doc) => doc,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}

// The checks below are free functions rather than inline bodies for one reason: the negative test
// at the bottom has to run the *same* code against a deliberately broken skill. A gate can only be
// trusted if something proves it fails.
//
// `assert!` with an explicit message rather than `assert_eq!` is deliberate too: assert_eq! panics
// with a non-String payload, and the negative test reads the message back to confirm the gate
// rejected the shape for the right reason.

fn check_documented_keys(path: &Path, doc: &SkillDoc) {
    for (key, _) in &doc.keys {
        assert!(
            ALLOWED_KEYS.contains(&key.as_str()),
            "{}: frontmatter key {key:?} is not one of {ALLOWED_KEYS:?}",
            path.display()
        );
    }
}

fn check_name_matches_directory(path: &Path, doc: &SkillDoc) {
    let name = match doc.keys.iter().find(|(k, _)| k == "name") {
        Some((_, v)) => v.clone(),
        None => panic!("{}: no `name:` in the frontmatter", path.display()),
    };
    assert!(
        !name.is_empty() && name.len() <= MAX_NAME_LEN,
        "{}: name {name:?} must be non-empty and at most {MAX_NAME_LEN} characters",
        path.display()
    );
    assert!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_'),
        "{}: name {name:?} must be lowercase letters, digits, '-' or '_'",
        path.display()
    );
    let dir = dir_name_of(path);
    assert!(
        name == dir,
        "{}: `name:` is {name:?} but the directory is {dir:?}; the loader and the router disagree",
        path.display()
    );
}

fn check_description(path: &Path, doc: &SkillDoc) {
    let description = match doc.keys.iter().find(|(k, _)| k == "description") {
        Some((_, v)) => v.clone(),
        None => panic!("{}: no `description:`", path.display()),
    };
    assert!(
        !description.is_empty(),
        "{}: an empty description means nothing routes to the skill",
        path.display()
    );
    assert!(
        description.len() <= MAX_DESCRIPTION_LEN,
        "{}: description is {} bytes, over the {MAX_DESCRIPTION_LEN} cap",
        path.display(),
        description.len()
    );
    assert!(
        !description.contains(':'),
        "{}: an unquoted description containing ':' needs quoting to stay one YAML scalar",
        path.display()
    );
}

fn check_has_body(path: &Path, doc: &SkillDoc) {
    assert!(
        !doc.body.trim().is_empty(),
        "{}: frontmatter with no instructions is a routable nothing",
        path.display()
    );
}

fn check_no_bookkeeping(path: &Path) {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) => panic!("cannot read {}: {e}", path.display()),
    };
    assert!(
        !raw.contains(INSTALLER_BOOKKEEPING),
        "{}: contains {INSTALLER_BOOKKEEPING:?}. Ownership belongs in the stamp file install.sh          writes beside the skill, not in a document two other programs parse.",
        path.display()
    );
}

/// Where the negative test parks its deliberately broken skill: inside `rust/tests/`, so the real
/// `skills/` scan never sees it and the two cannot interfere across cargo's test threads.
fn probe_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(".gate-probe")
}

/// The directory name has to equal the `name:` key the probes use, or the name/dir gate fires on
/// every case and the test proves nothing about the shape it was building.
fn probe_dir() -> PathBuf {
    probe_root().join("gate-probe")
}

/// F-15 and F-16 both survived suites that only ever asked "is the file there" and "does it
/// contain the marker". This builds each forbidden shape and requires the same gates to say no.
#[test]
fn negative_shapes_are_rejected_by_the_same_gates() {
    let dir = probe_dir();
    for (case, body, needle) in [
        (
            "comment_inside_fence",
            "---\n# managed by cortexyoung install.sh\nname: gate-probe\ndescription: probe\n---\n\nbody\n",
            "is a comment",
        ),
        (
            "fence_not_first",
            "# managed by cortexyoung install.sh\n---\nname: gate-probe\ndescription: probe\n---\n\nbody\n",
            "fence",
        ),
        (
            "bogus_key",
            "---\nname: gate-probe\ndescription: probe\ninstaller_owned: yes\n---\n\nbody\n",
            "frontmatter key",
        ),
        (
            "name_dir",
            "---\nname: other-name\ndescription: probe\n---\n\nbody\n",
            "the directory is",
        ),
        (
            "bad_description",
            "---\nname: gate-probe\ndescription: unquoted: colon\n---\n\nbody\n",
            "description",
        ),
        (
            "no_body",
            "---\nname: gate-probe\ndescription: probe\n---\n\n",
            "routable nothing",
        ),
    ] {
        let _ = fs::remove_dir_all(probe_root());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        fs::write(&path, body).unwrap();
        let outcome = std::panic::catch_unwind(|| {
            let doc = parse_or_panic(&path);
            check_documented_keys(&path, &doc);
            check_name_matches_directory(&path, &doc);
            check_description(&path, &doc);
            check_has_body(&path, &doc);
            check_no_bookkeeping(&path);
        });
        let _ = fs::remove_dir_all(probe_root());
        let err = match outcome {
            Ok(()) => panic!("{case}: the gates accepted a shape that must never deploy"),
            Err(e) => e
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| format!("<non-string panic: {e:?}>")),
        };
        assert!(
            err.contains(needle),
            "{case}: gate rejected for the wrong reason: {err}"
        );
    }
}
