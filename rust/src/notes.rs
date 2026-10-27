//! Local, pure library preparation for harness memory notes. No note leaves this module.
//! `project_for` attributes a subdirectory to its enclosing repository because git walks up;
//! this is intentional.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const MAX_NOTE_BYTES: u64 = 64 * 1024;
const MAX_SEGMENTS: usize = 32;
const MAX_CANDIDATES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanStatus {
    Read,
    SkippedSize,
    SkippedSymlink,
    SkippedNotFile,
    SkippedName,
    SkippedEncoding,
    SkippedUnreadable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub notes: Vec<ScannedNote>,
    /// False if any directory listing or directory entry failed.
    pub complete: bool,
    /// Encoded directories whose memory listing was read to the end without an error. Only
    /// these can prove a note is gone: a directory with no rows may still be one of them.
    pub listed_dirs: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedNote {
    pub encoded_dir_name: String,
    pub file_name: String,
    pub bytes: Option<Vec<u8>>,
    pub sha256: Option<String>,
    pub status: ScanStatus,
}

/// Snapshot direct `projects/*/memory/*.md` files. Directory and file read errors
/// become skipped rows; the scan never follows symlinks.
pub fn scan(claude_home: &Path) -> ScanReport {
    let mut rows = Vec::new();
    let mut complete = true;
    let mut listed_dirs = BTreeSet::new();
    let projects = claude_home.join("projects");
    let project_dirs = match fs::read_dir(&projects) {
        Ok(dirs) => dirs,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ScanReport {
                notes: rows,
                complete,
                listed_dirs,
            }
        }
        Err(error) => {
            return ScanReport {
                notes: vec![skipped("", "", error.to_string())],
                complete: false,
                listed_dirs,
            }
        }
    };
    for project in project_dirs {
        let project = match project {
            Ok(project) => project,
            Err(error) => {
                complete = false;
                rows.push(skipped("", "", error.to_string()));
                continue;
            }
        };
        let kind = match project.file_type() {
            Ok(kind) => kind,
            Err(error) => {
                complete = false;
                rows.push(skipped("", "", error.to_string()));
                continue;
            }
        };
        let raw_name = project.file_name();
        let encoded = raw_name.to_string_lossy().into_owned();
        if kind.is_symlink() {
            rows.push(status(&encoded, "", ScanStatus::SkippedSymlink));
            continue;
        }
        if !kind.is_dir() {
            continue;
        }
        if raw_name.to_str().is_none() {
            rows.push(status(&encoded, "", ScanStatus::SkippedName));
            continue;
        }
        let memory = project.path().join("memory");
        match fs::symlink_metadata(&memory) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(metadata) if metadata.file_type().is_symlink() => {
                rows.push(status(&encoded, "memory", ScanStatus::SkippedSymlink));
                continue;
            }
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                complete = false;
                rows.push(skipped(&encoded, "", error.to_string()));
                continue;
            }
        }
        let files = match fs::read_dir(&memory) {
            Ok(files) => files,
            Err(error) => {
                complete = false;
                rows.push(skipped(&encoded, "", error.to_string()));
                continue;
            }
        };
        let mut listed = true;
        for file in files {
            let file = match file {
                Ok(file) => file,
                Err(error) => {
                    complete = false;
                    listed = false;
                    rows.push(skipped(&encoded, "", error.to_string()));
                    continue;
                }
            };
            let raw_name = file.file_name();
            let name = raw_name.to_string_lossy().into_owned();
            if !name.ends_with(".md") {
                continue;
            }
            if raw_name.to_str().is_none() {
                rows.push(status(&encoded, &name, ScanStatus::SkippedName));
                continue;
            }
            let path = file.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    rows.push(skipped(&encoded, &name, error.to_string()));
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                rows.push(status(&encoded, &name, ScanStatus::SkippedSymlink));
                continue;
            }
            if !metadata.file_type().is_file() {
                rows.push(status(&encoded, &name, ScanStatus::SkippedNotFile));
                continue;
            }
            if metadata.len() > MAX_NOTE_BYTES {
                rows.push(ScannedNote {
                    encoded_dir_name: encoded.clone(),
                    file_name: name,
                    bytes: None,
                    sha256: None,
                    status: ScanStatus::SkippedSize,
                });
                continue;
            }
            // The metadata check is only an optimization. The read limit also bounds a
            // file that grows between metadata and open.
            let result = open_note(&path).and_then(|file| {
                if !file.metadata()?.is_file() {
                    return Err(std::io::Error::other("not a regular file after open"));
                }
                let mut bytes = Vec::new();
                file.take(MAX_NOTE_BYTES + 1).read_to_end(&mut bytes)?;
                Ok(bytes)
            });
            match result {
                Ok(bytes) if bytes.len() as u64 > MAX_NOTE_BYTES => rows.push(ScannedNote {
                    encoded_dir_name: encoded.clone(),
                    file_name: name,
                    bytes: None,
                    sha256: None,
                    status: ScanStatus::SkippedSize,
                }),
                Ok(bytes) if std::str::from_utf8(&bytes).is_err() => {
                    rows.push(status(&encoded, &name, ScanStatus::SkippedEncoding))
                }
                Ok(bytes) => rows.push(ScannedNote {
                    encoded_dir_name: encoded.clone(),
                    file_name: name,
                    sha256: Some(hex_digest(&bytes)),
                    bytes: Some(bytes),
                    status: ScanStatus::Read,
                }),
                Err(error) if is_symlink_open_error(&error) => {
                    rows.push(status(&encoded, &name, ScanStatus::SkippedSymlink))
                }
                Err(error) => rows.push(skipped(&encoded, &name, error.to_string())),
            }
        }
        if listed {
            listed_dirs.insert(encoded);
        }
    }
    rows.sort_by(|a, b| {
        (&a.encoded_dir_name, &a.file_name).cmp(&(&b.encoded_dir_name, &b.file_name))
    });
    ScanReport {
        notes: rows,
        complete,
        listed_dirs,
    }
}

fn status(encoded: &str, name: &str, status: ScanStatus) -> ScannedNote {
    ScannedNote {
        encoded_dir_name: encoded.into(),
        file_name: name.into(),
        bytes: None,
        sha256: None,
        status,
    }
}

fn open_note(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    options.custom_flags(0o400000 | 0o4000); // O_NOFOLLOW | O_NONBLOCK
    #[cfg(target_os = "macos")]
    options.custom_flags(0x100 | 0x4); // O_NOFOLLOW | O_NONBLOCK
    options.open(path)
}

fn is_symlink_open_error(error: &std::io::Error) -> bool {
    #[cfg(target_os = "linux")]
    {
        error.raw_os_error() == Some(40)
    } // ELOOP
    #[cfg(target_os = "macos")]
    {
        error.raw_os_error() == Some(62)
    } // ELOOP
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = error;
        false
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
    out
}

fn skipped(encoded: &str, name: &str, error: String) -> ScannedNote {
    ScannedNote {
        encoded_dir_name: encoded.into(),
        file_name: name.into(),
        bytes: None,
        sha256: None,
        status: ScanStatus::SkippedUnreadable(error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    Unique(PathBuf),
    Ambiguous(Vec<PathBuf>),
    Unknown,
    NotFound,
}

/// Reverse Claude Code's lossy path encoding only when exactly one existing
/// directory matches. Callers must match on the variant only: `Ambiguous`
/// means at least two real candidates; `Unknown` means traversal could not prove a result.
pub fn decode_project_dir(encoded: &str, fs_root: &Path) -> Decoded {
    if !encoded.starts_with('-') || encoded.len() < 2 || !fs_root.is_dir() {
        return Decoded::NotFound;
    }
    let mut pending = vec![(fs_root.to_path_buf(), String::new(), 0usize)];
    let mut matches = Vec::new();
    let mut candidates = 0usize;
    while let Some((path, prefix, depth)) = pending.pop() {
        if depth >= MAX_SEGMENTS {
            if encoded.starts_with(&prefix) && prefix != encoded {
                return Decoded::Unknown;
            }
            continue;
        }
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(_) => return Decoded::Unknown,
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return Decoded::Unknown,
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => return Decoded::Unknown,
            };
            let name = entry.file_name();
            let utf8 = match name.to_str() {
                Some(utf8) if !kind.is_symlink() => utf8,
                _ => {
                    if unknown_name_matches(&name, &prefix, encoded) {
                        return Decoded::Unknown;
                    }
                    continue;
                }
            };
            if !kind.is_dir() {
                continue;
            }
            let name = utf8;
            let next = format!("{prefix}-{}", encode_component(name));
            // A component must end where the encoding does or at a separator: "/ho" is not a
            // candidate for "-home-...", and counting it would push real names into the cap.
            if !encoded.starts_with(&next)
                || !(encoded.len() == next.len() || encoded[next.len()..].starts_with('-'))
            {
                continue;
            }
            candidates += 1;
            if candidates > MAX_CANDIDATES {
                return Decoded::Unknown;
            }
            let child = entry.path();
            if next == encoded {
                matches.push(child);
            } else {
                pending.push((child, next, depth + 1));
            }
        }
    }
    matches.sort();
    match matches.len() {
        0 => Decoded::NotFound,
        1 => Decoded::Unique(matches.remove(0)),
        _ => Decoded::Ambiguous(matches),
    }
}

fn unknown_name_matches(name: &std::ffi::OsStr, prefix: &str, encoded: &str) -> bool {
    // Lossy display is only a conservative lead: an undecodable name may still encode
    // to the current prefix, so never turn a possible match into `Unique`.
    let next = format!("{prefix}-{}", encode_component(&name.to_string_lossy()));
    encoded.starts_with(&next)
        && (encoded.len() == next.len() || encoded[next.len()..].starts_with('-'))
}

fn encode_component(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

pub fn project_for(dir: &Path) -> Result<String, String> {
    crate::memory_store::project_from_git_origin(dir)
}

/// Coarse, fail-closed credential screen. Rules: JWTs; GitHub, OpenAI/Stripe,
/// Slack and AWS key prefixes; PEM private keys; URL userinfo; Authorization and
/// Bearer headers; credential-named assignments; and high-entropy long runs.
/// It can miss secrets and can reject ordinary text; it is not a privacy proof.
pub fn looks_secret(body: &str) -> Option<&'static str> {
    for prefix in ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"] {
        if has_prefixed_run(body, prefix, 16) {
            return Some("github_token");
        }
    }
    for prefix in [
        "sk-", "sk_live_", "rk_live_", "whsec_", "hf_", "AIza", "ya29.", "fm1_", "fm2_",
    ] {
        if has_prefixed_run(body, prefix, 16) {
            return Some("api_token");
        }
    }
    for prefix in ["xoxa-", "xoxb-", "xoxp-", "xoxr-", "xapp-", "glpat-"] {
        if has_prefixed_run(body, prefix, 16) {
            return Some("slack_token");
        }
    }
    if body.contains("hooks.slack.com/services/") {
        return Some("slack_token");
    }
    if body.contains("FlyV1 ")
        && (has_prefixed_run(body, "fm1_", 16) || has_prefixed_run(body, "fm2_", 16))
    {
        return Some("api_token");
    }
    for prefix in ["AKIA", "ASIA"] {
        if has_prefixed_run(body, prefix, 16) {
            return Some("aws_access_key");
        }
    }
    if body
        .lines()
        .any(|line| line.contains("-----BEGIN ") && line.contains(" KEY-----"))
    {
        return Some("pem_key");
    }
    for word in body.split_whitespace() {
        if is_jwt(
            word.trim_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.'
            }),
        ) {
            return Some("jwt");
        }
        if let Some((_, rest)) = word.split_once("://") {
            if let Some((userinfo, _)) = rest.split_once('@') {
                // user:pass, :pass (redis://:pw@host) and a lone token (https://TOKEN@host);
                // a short login such as git@ is not a credential.
                if userinfo
                    .split_once(':')
                    .map_or(credential_value(userinfo), |(_, pass)| !pass.is_empty())
                {
                    return Some("url_userinfo");
                }
            }
            if let Some((_, parameters)) = rest.split_once(['?', '#']) {
                for parameter in parameters.split(['&', '#']) {
                    if let Some((key, value)) = parameter.split_once('=') {
                        if credential_key(key)
                            && !value.trim_matches(['"', '\'', '`', ')', ',']).is_empty()
                        {
                            return Some("url_query_secret");
                        }
                    }
                }
            }
        }
    }
    for line in body.lines() {
        if curl_userinfo(line) {
            return Some("curl_userinfo");
        }
        if let Some(rule) = assignments(line) {
            return Some(rule);
        }
    }
    if body
        .split(|c: char| !is_entropy_char(c))
        .any(high_entropy_run)
    {
        return Some("high_entropy");
    }
    // The longest-stretch rule above cannot see a base64 key that a / or + splits, such as an
    // AWS secret access key; judge base64 runs whole, but only when they hold one of those.
    if body
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')))
        .any(base64_run)
    {
        return Some("high_entropy");
    }
    None
}

fn base64_run(run: &str) -> bool {
    let core = run.trim_end_matches('=');
    core.len() >= 30
        && core.contains(['+', '/'])
        && !core.starts_with('/')
        && !core.contains("//")
        && core.bytes().any(|b| b.is_ascii_uppercase())
        && core.bytes().any(|b| b.is_ascii_lowercase())
        && core.bytes().filter(|b| b.is_ascii_digit()).count() >= 2
        && entropy(core) >= 4.5
}

fn entropy(run: &str) -> f64 {
    let mut counts = [0usize; 128];
    for b in run.bytes() {
        counts[(b & 127) as usize] += 1;
    }
    let len = run.len() as f64;
    counts
        .iter()
        .filter(|&&n| n != 0)
        .map(|&n| {
            let p = n as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=')
}
fn is_entropy_char(c: char) -> bool {
    is_token_char(c)
}

fn has_prefixed_run(body: &str, prefix: &str, min_following: usize) -> bool {
    body.match_indices(prefix).any(|(at, _)| {
        // A prefix starts a word: "ask-user-question" does not contain an "sk-" key.
        if body[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return false;
        }
        body[at + prefix.len()..]
            .chars()
            .take_while(|c| is_token_char(*c))
            .count()
            >= min_following
    })
}

fn is_jwt(word: &str) -> bool {
    let parts: Vec<_> = word.split('.').collect();
    parts.len() == 3
        && word.len() >= 40
        && parts[0].starts_with("eyJ")
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

fn credential_key(key: &str) -> bool {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut previous_lower = false;
    for ch in key.chars() {
        if matches!(ch, '_' | '-' | '.') || !ch.is_ascii_alphanumeric() {
            if !part.is_empty() {
                parts.push(std::mem::take(&mut part));
            }
            previous_lower = false;
        } else {
            if ch.is_ascii_uppercase() && previous_lower && !part.is_empty() {
                parts.push(std::mem::take(&mut part));
            }
            part.push(ch.to_ascii_lowercase());
            previous_lower = ch.is_ascii_lowercase();
        }
    }
    if !part.is_empty() {
        parts.push(part);
    }
    let has = |needle: &str| parts.iter().any(|part| part == needle);
    parts.iter().any(|part| {
        [
            "token",
            "secret",
            "password",
            "passwd",
            "pwd",
            "apikey",
            "auth",
            "credential",
            "credentials",
            "cookie",
            "session",
            "dsn",
            "webhook",
            "signature",
            "sig",
        ]
        .contains(&part.as_str())
    }) || [
        ("api", "key"),
        ("access", "key"),
        ("private", "key"),
        ("client", "secret"),
        ("signing", "key"),
        ("encryption", "key"),
        ("database", "url"),
    ]
    .iter()
    .any(|(a, b)| has(a) && has(b))
}

fn curl_userinfo(line: &str) -> bool {
    let words: Vec<_> = line.split_whitespace().collect();
    words.windows(2).any(|pair| {
        matches!(pair[0], "-u" | "--user")
            && pair[1]
                .trim_matches(['"', '\''])
                .split_once(':')
                .is_some_and(|(user, pass)| !user.is_empty() && credential_value(pass))
    })
}

fn assignments(line: &str) -> Option<&'static str> {
    let bytes = line.as_bytes();
    for (i, &separator) in bytes.iter().enumerate() {
        if separator != b'=' && separator != b':' {
            continue;
        }
        // "auth.rs::email_delivery" is a path, not an assignment.
        if separator == b':' && (bytes.get(i + 1) == Some(&b':') || (i > 0 && bytes[i - 1] == b':'))
        {
            continue;
        }
        let mut end = i;
        while end > 0 && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        // A quoted key, as in JSON: "password": "...".
        let quote =
            (end > 0 && matches!(bytes[end - 1], b'"' | b'\'' | b'`')).then(|| bytes[end - 1]);
        if quote.is_some() {
            end -= 1;
        }
        let mut start = end;
        while start > 0
            && (bytes[start - 1].is_ascii_alphanumeric()
                || matches!(bytes[start - 1], b'_' | b'-' | b'.'))
        {
            start -= 1;
        }
        if start == end
            || (start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_'))
        {
            continue;
        }
        let key = &line[start..end];
        // A colon assigns only where a key can stand: at the start of a line, after a list
        // marker, or inside a structure (YAML, a header, JSON). Mid-sentence, "from the same
        // session: [[a-note]]" is prose.
        if separator == b':' {
            let mut before = start;
            if quote.is_some() && before > 0 && Some(bytes[before - 1]) == quote {
                before -= 1;
            }
            while before > 0 && bytes[before - 1].is_ascii_whitespace() {
                before -= 1;
            }
            if before > 0
                && !matches!(
                    bytes[before - 1],
                    b'"' | b'\''
                        | b'`'
                        | b'{'
                        | b'('
                        | b'['
                        | b','
                        | b'-'
                        | b'*'
                        | b'|'
                        | b'>'
                        | b'#'
                        | b';'
                )
                && !after_flag(&line[..before])
            {
                continue;
            }
        }
        let header = separator == b':'
            && [
                "authorization",
                "proxy-authorization",
                "x-api-key",
                "x-auth-token",
                "api-key",
                "cookie",
                "set-cookie",
            ]
            .contains(&key.to_ascii_lowercase().as_str());
        if !header && !credential_key(key) {
            continue;
        }
        let mut value = line[i + 1..].trim_start();
        if header
            && (key.eq_ignore_ascii_case("authorization")
                || key.eq_ignore_ascii_case("proxy-authorization"))
        {
            if let Some((scheme, rest)) = value.split_once(char::is_whitespace) {
                if scheme.chars().all(|c| c.is_ascii_alphabetic()) {
                    value = rest.trim_start();
                }
            }
        }
        if header && (key.eq_ignore_ascii_case("cookie") || key.eq_ignore_ascii_case("set-cookie"))
        {
            if let Some((_, rest)) = value.split_once('=') {
                value = rest;
            }
        }
        let candidate = if value.starts_with('"') || value.starts_with('\'') {
            let quote = value.as_bytes()[0] as char;
            match value[1..].find(quote) {
                Some(end) => &value[..end + 2],
                None => value,
            }
        } else {
            value.split_whitespace().next().unwrap_or("")
        };
        // Claude Code stamps `originSessionId: <uuid>` on its own notes; a session id that is a
        // transcript id, not a login. Only that key: a `session_id: <uuid>` may well be a bearer.
        if !header
            && key.eq_ignore_ascii_case("originSessionId")
            && is_uuid(candidate.trim_matches(['"', '\'', '`', ',']))
        {
            continue;
        }
        if credential_value(candidate) {
            return Some(if header {
                "authorization"
            } else {
                "credential_assignment"
            });
        }
    }
    None
}

/// True when `head` ends in a command-line flag word such as `-H` or `--header`, as in
/// `curl -H X-Api-Key: ...`.
fn after_flag(head: &str) -> bool {
    head.split_whitespace()
        .next_back()
        .is_some_and(|word| word.starts_with('-') && word.len() > 1)
}

fn is_uuid(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    parts.iter().map(|part| part.len()).eq([8, 4, 4, 4, 12])
        && parts
            .iter()
            .all(|part| part.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn credential_value(value: &str) -> bool {
    let raw = value
        .split_once(" # ")
        .map_or(value, |(before, _)| before)
        .trim();
    // Credentials are ASCII: "sid、permission=..." ends at the first non-ASCII character.
    let raw = raw
        .find(|c: char| !c.is_ascii())
        .map_or(raw, |end| &raw[..end]);
    let quoted = raw.len() >= 2
        && ((raw.starts_with('"') && raw.ends_with('"'))
            || (raw.starts_with('\'') && raw.ends_with('\'')));
    let value = raw.trim_matches(|c: char| {
        matches!(c, '"' | '\'' | '`' | ',' | ')' | ';' | '.') || !c.is_ascii()
    });
    // URLs and paths are judged by the URL rules, not as a bare value.
    if value.contains("://") || value.starts_with('/') {
        return false;
    }
    // References and elisions name a secret without holding one: $TOKEN, ${TOKEN},
    // $(armo ...), cfat_..., <token>. A $2b$... or $argon2id$... hash is not a reference.
    let reference = value.strip_prefix('$').is_some_and(|rest| {
        (rest.starts_with('{') && rest.ends_with('}'))
            || rest.starts_with('(')
            || (rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    });
    let named_placeholder = value.len() > 2
        && value.starts_with('<')
        && value.ends_with('>')
        && value[1..value.len() - 1]
            .chars()
            .all(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '-' | ' '));
    if reference || named_placeholder || value.contains("...") {
        return false;
    }
    // A credential is one unbroken value; "6000 bytes; the password prompt ..." is prose.
    if !quoted && value.split_whitespace().count() > 1 {
        return false;
    }
    value.len() >= 8
        && (value
            .bytes()
            .any(|b| b.is_ascii_digit() || b.is_ascii_uppercase() || !b.is_ascii_alphanumeric()))
        && !(value.is_empty()
            || value == "..."
            || value.eq_ignore_ascii_case("xxx")
            || [
                "changeme", "redacted", "required", "none", "null", "true", "false", "empty",
                "unset",
            ]
            .iter()
            .any(|word| value.eq_ignore_ascii_case(word)))
}

fn high_entropy_run(run: &str) -> bool {
    if ["/", "~/", "./", "../"]
        .iter()
        .any(|prefix| run.starts_with(prefix))
    {
        return false;
    }
    // Judge the longest alphanumeric stretch: hyphenated or slashed names such as
    // "stage3-expand-itinerary/SKILL" are words, and generated keys are mostly one stretch.
    let run = run
        .split(|c: char| !c.is_ascii_alphanumeric())
        .max_by_key(|part| part.len())
        .unwrap_or("");
    // Generated keys mix cases; the slugs, dates and hex digests that fill notes are one case.
    let mixed =
        run.bytes().any(|b| b.is_ascii_uppercase()) && run.bytes().any(|b| b.is_ascii_lowercase());
    if run.len() < 20
        || !mixed
        || run.bytes().filter(|b| b.is_ascii_digit()).count() < 2
        || run.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return false;
    }
    entropy(run) >= 4.0
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub kind: Option<String>,
}

/// Read only the initial YAML-style scalar block. These values are untrusted
/// display data, so controls are replaced, whitespace collapsed and the
/// description is limited to 200 characters; name and kind to 100.
pub fn frontmatter(body: &str) -> Frontmatter {
    let lines: Vec<_> = body.lines().collect();
    if lines.first().copied() != Some("---") {
        return Frontmatter::default();
    }
    let mut result = Frontmatter::default();
    let mut in_metadata = false;
    let mut closed = false;
    let mut index = 1;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            in_metadata = false;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let key = key.trim_end();
        let mut value = value.trim().to_string();
        if indent == 0
            && key == "description"
            && matches!(value.as_str(), ">" | "|" | ">-" | "|-" | ">+" | "|+")
        {
            let mut pieces = Vec::new();
            while index < lines.len() {
                let next = lines[index];
                if !next.trim().is_empty() && next.len() - next.trim_start().len() <= indent {
                    break;
                }
                if !next.trim().is_empty() {
                    pieces.push(next.trim());
                }
                index += 1;
            }
            value = pieces.join(" ");
        }
        value = value
            .split_once(" # ")
            .map_or(value.as_str(), |(before, _)| before)
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .to_string();
        if indent == 0 && key == "metadata" {
            in_metadata = true;
        } else if indent == 0 && key == "name" {
            result.name = nonempty(sanitize(&value, Some(100)));
        } else if indent == 0 && key == "description" {
            result.description = nonempty(sanitize(&value, Some(200)));
        } else if indent > 0 && in_metadata && key == "type" {
            result.kind = nonempty(sanitize(&value, Some(100)));
        }
    }
    if closed {
        result
    } else {
        Frontmatter::default()
    }
}

fn nonempty(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn sanitize(value: &str, cap: Option<usize>) -> String {
    let replaced: String = value
        .chars()
        .map(|c| if c.is_control() || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}') { ' ' } else { c })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    match cap {
        Some(cap) => collapsed.chars().take(cap).collect(),
        None => collapsed,
    }
}
