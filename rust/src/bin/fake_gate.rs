//! Subprocess fixture for the optional hook resource gate.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("allow") => {}
        Some("deny") => {
            println!("{}", args.get(2).map(String::as_str).unwrap_or("denied"));
            std::process::exit(75);
        }
        Some("deny-empty") => std::process::exit(75),
        Some("fail") => std::process::exit(1),
        Some("sleep") => {
            if let Some(path) = args.get(3) {
                let _ = std::fs::write(path, std::process::id().to_string());
            }
            let ms = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
            std::thread::sleep(Duration::from_millis(ms));
        }
        Some("ignore-stdin") => {
            let ms = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);
            std::thread::sleep(Duration::from_millis(ms));
        }
        Some("flood") => {
            let _ = std::io::stdout().write_all(&vec![b'x'; 8192]);
            std::process::exit(75);
        }
        Some("abort") => std::process::abort(),
        Some("grandchild") => {
            if let Ok(exe) = std::env::current_exe() {
                let _ = Command::new(exe)
                    .args(["sleep", "10000"])
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
        _ => std::process::exit(2),
    }
}
