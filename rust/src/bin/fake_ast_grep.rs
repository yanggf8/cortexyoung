//! A scripted stand-in for the `ast-grep` CLI, driven by `FAKE_AG_MODE`.
//!
//! Dev-only test fixture: it lets the pathological parser paths (timeout, non-zero exit,
//! malformed-but-parseable scan lines, pre-flight ERROR nodes) be tested without depending on a
//! real ast-grep misbehaving, and without depending on the host having any scripting runtime.
//!
//! It is *never* shipped: `install.sh` copies `target/release/cort` and the pack only, and
//! `tests/install-smoke.sh` asserts the installed payload directory holds nothing else.
//!
//! Mode grammar (kept byte-compatible with the python fixture it replaced, so the tests that set
//! `FAKE_AG_MODE` did not have to change):
//!
//!   ""                -> exit 0, no output
//!   version:<x.y.z>   -> stdout `ast-grep <x.y.z>`, exit 0 (also the default for `--version`)
//!   hang              -> sleep 60s (exercises the subprocess timeout)
//!   streams           -> stdout `OUT\n`, stderr `ERR\n`, exit 1
//!   empty             -> exit 1, no output (ast-grep's zero-match *and* bad-pattern shape)
//!   emit:<base64>     -> stdout = decoded bytes, exit 0
//!   preflight-bad     -> stderr with `Debug AST:` + `ERROR node` warning, exit 0
//!   preflight-ok      -> stderr with `Debug AST:` only, exit 0

use std::io::Write;
use std::time::Duration;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding, no whitespace tolerance.
///
/// Deliberately strict: a fixture that quietly accepts a malformed payload would let a
/// malformed-stream test pass by emitting nothing, which is the opposite of what it means to
/// assert. The payloads come from this repo's own `base64_encode` helpers, so anything rejected
/// here is a bug in the test.
fn b64_decode(input: &str) -> Option<Vec<u8>> {
    if !input.len().is_multiple_of(4) {
        return None;
    }
    let pad = input.bytes().rev().take_while(|&b| b == b'=').count();
    if pad > 2 {
        return None;
    }
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for c in input[..input.len() - pad].chars() {
        let value = ALPHABET.iter().position(|&a| a == c as u8)? as u32;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if bits >= 6 {
        return None; // a dangling 6-bit group is not a valid encoding
    }
    Some(out)
}

fn main() {
    let mode = std::env::var("FAKE_AG_MODE").unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.first().map(String::as_str) == Some("--version") {
        let version = mode.strip_prefix("version:").unwrap_or("0.45.2");
        println!("ast-grep {version}");
        return;
    }

    match mode.as_str() {
        "hang" => {
            std::thread::sleep(Duration::from_secs(60));
            std::process::exit(0);
        }
        "streams" => {
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            let _ = stdout.write_all(b"OUT\n").and_then(|_| stdout.flush());
            let _ = stderr.write_all(b"ERR\n").and_then(|_| stderr.flush());
            std::process::exit(1);
        }
        "empty" => std::process::exit(1),
        "preflight-bad" => {
            eprint!(
                "Debug AST:\nprogram (0,0)-(0,10)\n  ERROR (0,0)-(0,10)\n\n\
Warning: Pattern contains an ERROR node and may cause unexpected results.\n"
            );
            std::process::exit(0);
        }
        "preflight-ok" => {
            eprint!("Debug AST:\nprogram (0,0)-(0,9)\n");
            std::process::exit(0);
        }
        _ => {}
    }

    if let Some(payload) = mode.strip_prefix("emit:") {
        let bytes = match b64_decode(payload) {
            Some(b) => b,
            None => std::process::exit(2),
        };
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        let _ = stdout.write_all(&bytes).and_then(|_| stdout.flush());
        std::process::exit(0);
    }

    std::process::exit(0);
}
