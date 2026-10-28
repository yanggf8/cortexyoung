//! Local configuration for the optional external resource gate.

use crate::data_dir::{data_dir, write_atomic};
use crate::errors::CortError;
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT_MS: u64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateConfig {
    Unconfigured,
    Invalid {
        reason: String,
    },
    Configured {
        command: Vec<String>,
        timeout_ms: u64,
    },
}

pub fn path() -> PathBuf {
    data_dir().join("gate.json")
}

fn validate(command: &[String], timeout_ms: u64) -> Result<(), String> {
    let Some(first) = command.first() else {
        return Err("command must be a nonempty array".into());
    };
    if !Path::new(first).is_absolute() {
        return Err("command[0] must be an absolute path".into());
    }
    if !(50..=1000).contains(&timeout_ms) {
        return Err("timeout_ms must be between 50 and 1000".into());
    }
    Ok(())
}

fn parse(value: Value) -> Result<GateConfig, String> {
    let object = value.as_object().ok_or("gate config must be an object")?;
    if object.get("v").and_then(Value::as_u64) != Some(1) {
        return Err("v must be 1".into());
    }
    let values = object
        .get("command")
        .and_then(Value::as_array)
        .ok_or("command must be an array")?;
    let command: Vec<String> = values
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or("command entries must be strings")
        })
        .collect::<Result<_, _>>()?;
    let timeout_ms = match object.get("timeout_ms") {
        Some(v) => v.as_u64().ok_or("timeout_ms must be an unsigned integer")?,
        None => DEFAULT_TIMEOUT_MS,
    };
    validate(&command, timeout_ms)?;
    Ok(GateConfig::Configured {
        command,
        timeout_ms,
    })
}

pub fn load() -> GateConfig {
    let body = match fs::read(path()) {
        Ok(body) => body,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return GateConfig::Unconfigured,
        Err(e) => {
            return GateConfig::Invalid {
                reason: format!("cannot read gate.json: {e}"),
            }
        }
    };
    match serde_json::from_slice::<Value>(&body)
        .map_err(|e| format!("invalid JSON: {e}"))
        .and_then(parse)
    {
        Ok(config) => config,
        Err(reason) => GateConfig::Invalid { reason },
    }
}

pub fn save(command: Vec<String>, timeout_ms: Option<u64>) -> Result<PathBuf, CortError> {
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    validate(&command, timeout_ms)
        .map_err(|reason| CortError::new("gate_config_invalid", json!({ "reason": reason })))?;
    let path = path();
    let body = json!({ "v": 1, "command": command, "timeout_ms": timeout_ms }).to_string();
    write_atomic(&path, &body).map_err(|e| {
        CortError::new(
            "gate_storage_error",
            json!({ "path": path, "message": e.to_string() }),
        )
    })?;
    Ok(path)
}

pub fn clear() -> io::Result<bool> {
    match fs::remove_file(path()) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub struct HandOff {
    pub tag: &'static str,
    pub elapsed_ms: Option<u64>,
    pub deny_reason: Option<String>,
}

const EMPTY_DENY_REASON: &str = "resource gate denied this command (no reason given)";

fn result_for_exit(code: Option<i32>, stdout: &[u8], elapsed_ms: u64) -> HandOff {
    let (tag, deny_reason) = match code {
        Some(0) => ("allow", None),
        Some(75) => {
            let reason = String::from_utf8_lossy(stdout);
            let reason = reason.trim();
            (
                "deny",
                Some(if reason.is_empty() {
                    EMPTY_DENY_REASON.to_owned()
                } else {
                    reason.to_owned()
                }),
            )
        }
        _ => ("error", None),
    };
    HandOff {
        tag,
        elapsed_ms: Some(elapsed_ms),
        deny_reason,
    }
}

/// Pass the original hook payload to a configured local gate. All failures allow the tool call.
pub fn hand_off(config: &GateConfig, payload: &[u8]) -> HandOff {
    let GateConfig::Configured {
        command,
        timeout_ms,
    } = config
    else {
        return HandOff {
            tag: match config {
                GateConfig::Unconfigured => "unconfigured",
                _ => "config_invalid",
            },
            elapsed_ms: None,
            deny_reason: None,
        };
    };
    let Some(executable) = command.first() else {
        return HandOff {
            tag: "config_invalid",
            elapsed_ms: None,
            deny_reason: None,
        };
    };
    let started = Instant::now();
    let mut child = match Command::new(executable)
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            return HandOff {
                tag: "spawn_failed",
                elapsed_ms: None,
                deny_reason: None,
            }
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let body = payload.to_vec();
        let _ = std::thread::Builder::new()
            .name("gate-stdin".into())
            .spawn(move || {
                let _ = stdin.write_all(&body);
            });
    }
    let (sender, receiver) = mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = std::thread::Builder::new()
            .name("gate-stdout".into())
            .spawn(move || {
                let mut kept = Vec::with_capacity(4096);
                let mut buf = [0u8; 4096];
                while let Ok(n) = stdout.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let remaining = 4096 - kept.len();
                    kept.extend_from_slice(&buf[..n.min(remaining)]);
                }
                let _ = sender.send(kept);
            });
    }
    let deadline = Duration::from_millis(*timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let remaining = deadline
                    .saturating_sub(started.elapsed())
                    .max(Duration::from_millis(5));
                let stdout = receiver.recv_timeout(remaining).unwrap_or_default();
                return result_for_exit(status.code(), &stdout, elapsed_ms);
            }
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(3))
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return HandOff {
                    tag: "timeout",
                    elapsed_ms: Some(started.elapsed().as_millis() as u64),
                    deny_reason: None,
                };
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return HandOff {
                    tag: "error",
                    elapsed_ms: Some(started.elapsed().as_millis() as u64),
                    deny_reason: None,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_mapping() {
        assert_eq!(result_for_exit(Some(0), b"ignored", 1).tag, "allow");
        assert_eq!(result_for_exit(Some(1), b"ignored", 1).tag, "error");
        assert_eq!(result_for_exit(None, b"ignored", 1).tag, "error");
        assert_eq!(
            result_for_exit(Some(75), b" \n", 1).deny_reason.as_deref(),
            Some(EMPTY_DENY_REASON)
        );
        assert_eq!(
            result_for_exit(Some(75), b" denied \n", 1)
                .deny_reason
                .as_deref(),
            Some("denied")
        );
    }

    #[test]
    fn invalid_shapes() {
        let cases = [
            "null",
            "[]",
            "{}",
            "{\"v\":2,\"command\":[\"/bin/x\"]}",
            "{\"v\":\"1\",\"command\":[\"/bin/x\"]}",
            "{\"v\":1}",
            "{\"v\":1,\"command\":[]}",
            "{\"v\":1,\"command\":[\"relative\"]}",
            "{\"v\":1,\"command\":\"/bin/x\"}",
            "{\"v\":1,\"command\":[3]}",
            "{\"v\":1,\"command\":[\"/bin/x\",false]}",
            "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":49}",
            "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":1001}",
            "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":\"200\"}",
            "{\"v\":1,\"command\":[\"/bin/x\"],\"timeout_ms\":null}",
        ];
        for case in cases {
            assert!(
                parse(serde_json::from_str(case).unwrap()).is_err(),
                "{case}"
            );
        }
    }

    #[test]
    fn default_and_bounds() {
        let config = parse(json!({"v":1,"command":["/bin/x"]})).unwrap();
        assert_eq!(
            config,
            GateConfig::Configured {
                command: vec!["/bin/x".into()],
                timeout_ms: 200
            }
        );
        assert!(parse(json!({"v":1,"command":["/bin/x"],"timeout_ms":50})).is_ok());
        assert!(parse(json!({"v":1,"command":["/bin/x"],"timeout_ms":1000})).is_ok());
    }
}
