use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

pub const USAGE: &str = "usage: cort-evals gate [--crate rust|evals] [--skip-tests] [--release]";

#[derive(Debug, PartialEq, Eq)]
struct Step {
    krate: &'static str,
    name: &'static str,
    args: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
struct Plan {
    steps: Vec<Step>,
    jobs: OsString,
    threads: OsString,
}

fn plan(
    args: &[String],
    jobs: Option<OsString>,
    threads: Option<OsString>,
) -> Result<Plan, String> {
    let mut selected = None;
    let mut skip_tests = false;
    let mut release = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--crate" if selected.is_none() => {
                selected = Some(match iter.next().map(String::as_str) {
                    Some("rust") => "rust",
                    Some("evals") => "evals",
                    _ => return Err(format!("--crate requires rust or evals\n{USAGE}")),
                });
            }
            "--skip-tests" if !skip_tests => skip_tests = true,
            "--release" if !release => release = true,
            _ => return Err(format!("unexpected argument {arg}\n{USAGE}")),
        }
    }
    let mut steps = Vec::new();
    for krate in ["rust", "evals"] {
        if selected.is_some_and(|s| s != krate) {
            continue;
        }
        steps.push(Step {
            krate,
            name: "fmt",
            args: &["fmt", "--all", "--", "--check"],
        });
        steps.push(Step {
            krate,
            name: "clippy",
            args: &[
                "clippy",
                "--locked",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
        });
        if !skip_tests {
            steps.push(Step {
                krate,
                name: "test",
                args: &["test", "--locked", "--all-targets"],
            });
        }
        // CI's release build is opt-in: it is the heaviest step and rarely fails alone.
        if release {
            steps.push(Step {
                krate,
                name: "release",
                args: &["build", "--release", "--locked"],
            });
        }
    }
    Ok(Plan {
        steps,
        jobs: jobs.unwrap_or_else(|| "2".into()),
        threads: threads.unwrap_or_else(|| "2".into()),
    })
}

fn repo_root() -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("git rev-parse: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let path = String::from_utf8(output.stdout).map_err(|e| format!("git output: {e}"))?;
    Ok(PathBuf::from(path.trim_end_matches(['\r', '\n'])))
}

pub fn run(args: &[String]) -> i32 {
    if args == ["--help"] || args == ["-h"] {
        println!("{USAGE}");
        return 0;
    }
    let plan = match plan(
        args,
        std::env::var_os("CARGO_BUILD_JOBS"),
        std::env::var_os("RUST_TEST_THREADS"),
    ) {
        Ok(plan) => plan,
        Err(error) => return setup_failure(error),
    };
    let root = match repo_root() {
        Ok(root) => root,
        Err(error) => return setup_failure(error),
    };
    let nice = cort_evals::arms::resolve_binary("nice");
    let mut completed: Vec<Value> = Vec::new();
    for step in &plan.steps {
        let mut command = match &nice {
            Some(path) => {
                let mut cmd = Command::new(path);
                cmd.args(["-n", "15", "cargo"]);
                cmd
            }
            None => Command::new("cargo"),
        };
        let start = Instant::now();
        let result = command
            .args(step.args)
            .current_dir(root.join(step.krate))
            .env("CARGO_BUILD_JOBS", &plan.jobs)
            .env("RUST_TEST_THREADS", &plan.threads)
            .status();
        completed.push(json!({"name": step.name, "crate": step.krate, "elapsed_ms": start.elapsed().as_millis()}));
        let (exit, error) = match result {
            Ok(status) if status.success() => continue,
            Ok(status) => (status.code().unwrap_or(1), None),
            Err(error) => (1, Some(error.to_string())),
        };
        let mut report = json!({"gate":"failed", "crate":step.krate, "step":step.name, "exit":exit, "steps":completed});
        if let Some(error) = error {
            report["error"] = json!(error);
        }
        println!("{report}");
        return 1;
    }
    println!("{}", json!({"gate":"ok", "steps":completed}));
    0
}

fn setup_failure(error: String) -> i32 {
    println!(
        "{}",
        json!({"gate":"failed", "crate":null, "step":"setup", "exit":1, "steps":[], "error":error})
    );
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn full_plan_orders_crates_and_ci_steps() {
        let plan = plan(&[], None, None).unwrap();
        let order: Vec<_> = plan.steps.iter().map(|s| (s.krate, s.name)).collect();
        assert_eq!(
            order,
            [
                ("rust", "fmt"),
                ("rust", "clippy"),
                ("rust", "test"),
                ("evals", "fmt"),
                ("evals", "clippy"),
                ("evals", "test")
            ]
        );
        assert_eq!(plan.jobs, "2");
        assert_eq!(plan.threads, "2");
        assert_eq!(
            plan.steps[1].args,
            [
                "clippy",
                "--locked",
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings"
            ]
        );
        assert_eq!(plan.steps[2].args, ["test", "--locked", "--all-targets"]);
        assert_eq!(plan.steps[0].args, ["fmt", "--all", "--", "--check"]);
    }

    #[test]
    fn selection_and_environment_are_preserved() {
        for krate in ["rust", "evals"] {
            let plan = plan(
                &args(&["--crate", krate, "--skip-tests"]),
                Some("7".into()),
                Some("3".into()),
            )
            .unwrap();
            assert_eq!(
                plan.steps
                    .iter()
                    .map(|s| (s.krate, s.name))
                    .collect::<Vec<_>>(),
                [(krate, "fmt"), (krate, "clippy")]
            );
            assert_eq!(plan.jobs, "7");
            assert_eq!(plan.threads, "3");
        }
        assert_eq!(
            plan(&args(&["--skip-tests"]), None, None)
                .unwrap()
                .steps
                .len(),
            4
        );
        let release = plan(&args(&["--crate", "rust", "--release"]), None, None).unwrap();
        assert_eq!(
            release.steps.iter().map(|s| s.name).collect::<Vec<_>>(),
            ["fmt", "clippy", "test", "release"]
        );
        assert_eq!(release.steps[3].args, ["build", "--release", "--locked"]);
    }

    #[test]
    fn invalid_options_are_rejected() {
        for a in [
            &["--crate"][..],
            &["--crate", "other"],
            &["--skip-tests", "extra"],
            &["--crate", "rust", "--crate", "evals"],
            &["--release", "--release"],
            &["--unknown"],
        ] {
            assert!(plan(&args(a), None, None).is_err());
        }
    }
}
