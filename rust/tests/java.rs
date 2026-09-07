//! The Java pack: owner-carrying method chunks, the three call forms, and the receiver gate
//! answering `impact` end to end. Same split as the Rust pack tests (context.rs holds the pack
//! shape, cli.rs holds the graph), but one language one file, because every fixture is Java.

use cort::ast_grep::resolve_ast_grep_bin;
use cort::chunker::{extract_file, ExtractFileArgs};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE: &str = concat!(
    "package demo;\n",
    "import demo.model.Order;\n",
    "public class OrderService {\n",
    "  private final Repo repo;\n",
    "  public OrderService(Repo repo) { this.repo = repo; }\n",
    "  public void save(Order o) {\n",
    "    repo.persist(o);\n",
    "    this.log();\n",
    "  }\n",
    "  void log() { Order local = new Order(); }\n",
    "}\n",
);

fn extract_real(abs: &Path, file_path: &str, source: &str) -> cort::chunker::ExtractResult {
    let bin = resolve_ast_grep_bin().expect("ast-grep on PATH");
    extract_file(ExtractFileArgs {
        bin: &bin,
        project_id: "p",
        file_path,
        abs_path: abs.to_str().unwrap(),
        source,
        timeout_ms: None,
    })
    .expect("extract")
}

fn extract_fixture(name: &str, body: &str) -> cort::chunker::ExtractResult {
    let dir = tempfile::tempdir().unwrap();
    let abs = dir.path().join(name);
    fs::create_dir_all(abs.parent().unwrap()).unwrap();
    fs::write(&abs, body).unwrap();
    extract_real(&abs, name, body)
}

fn symbols(r: &cort::chunker::ExtractResult) -> Vec<String> {
    let mut v: Vec<String> = r
        .chunks
        .iter()
        .filter_map(|c| c.symbol_name.clone())
        .collect();
    v.sort();
    v
}

fn has_edge(r: &cort::chunker::ExtractResult, rel: &str, form: &str, target: &str) -> bool {
    r.edges
        .iter()
        .any(|e| e.rel_type == rel && e.call_form.as_str() == form && e.raw_target == target)
}

/// The OWNER capture is what the receiver gate's first rule consumes, so a Java method chunk
/// without its owner would be a rule that extracts symbols `impact` can never bind.
#[test]
fn java_methods_carry_owners_and_types_are_chunks() {
    let r = extract_fixture("src/OrderService.java", SERVICE);
    assert_eq!(
        symbols(&r),
        vec![
            "OrderService",
            "OrderService::OrderService",
            "OrderService::log",
            "OrderService::save",
        ]
    );
    for c in &r.chunks {
        if c.symbol_name.as_deref() == Some("OrderService::save") {
            assert_eq!(c.chunk_type, "method");
            assert_eq!(c.language.as_deref(), Some("Java"));
        }
    }
}

/// `repo.persist(o)` has no head-only AST node the way Rust's `field_expression` is; the rule
/// captures `$OBJECT` + `$METHOD` and the chunker composes the stored head. `this.log()` binds
/// through the same self-scoping rule `self.m()` has in Rust.
#[test]
fn receiver_edges_store_composed_heads_and_the_edge_line_names_the_method() {
    let r = extract_fixture("src/OrderService.java", SERVICE);
    assert!(has_edge(&r, "calls", "receiver", "repo.persist"));
    assert!(has_edge(&r, "calls", "receiver", "this.log"));
    let e = r
        .edges
        .iter()
        .find(|e| e.raw_target == "repo.persist")
        .unwrap();
    assert_eq!(e.source_symbol.as_deref(), Some("OrderService::save"));
    // The head starts on line 7's indentation, but `persist` is named there too; what the edge may
    // never carry is the `public void save(Order o) {` line the invocation does not start on.
    assert_eq!(e.start_line, 7);
}

#[test]
fn imports_new_and_type_references_arrive_in_their_own_shapes() {
    let r = extract_fixture("src/OrderService.java", SERVICE);
    assert!(has_edge(&r, "imports", "bare", "demo.model.Order"));
    // `new` is a bare call: several classes named `Order` must land AMBIGUOUS, like a bare call.
    assert!(has_edge(&r, "calls", "bare", "Order"));
    // The parameter type is a reference edge; the field declaration `Repo repo` contributes the
    // same shape for `Repo`.
    assert!(has_edge(&r, "references", "type", "Order"));
    assert!(has_edge(&r, "references", "type", "Repo"));
    // Primitives are distinct node kinds in this grammar and never enter the graph.
    assert!(!r
        .edges
        .iter()
        .any(|e| e.raw_target == "int" || e.raw_target == "void"));
}

// ── end to end: index a Java project, ask impact ────────────────────────────────────────────

fn cort_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cort"))
}

fn run_cort(args: &[&str], cwd: &Path, cache: &Path) -> Run {
    let out = Command::new(cort_bin())
        .args(args)
        .current_dir(cwd)
        .env("CORT_CACHE_DIR", cache)
        .output()
        .expect("spawn cort");
    Run {
        code: out.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn payload(run: &Run) -> Value {
    serde_json::from_str(run.stdout.trim_end()).unwrap_or_else(|e| {
        panic!(
            "json parse failed: {e}; stdout={:?} stderr={:?}",
            run.stdout, run.stderr
        )
    })
}

fn java_sandbox(
    files: &[(&str, &str)],
) -> (tempfile::TempDir, PathBuf, tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cort-java-")
        .tempdir()
        .unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, body).unwrap();
    }
    let root = fs::canonicalize(dir.path()).unwrap();
    let cache_dir = tempfile::Builder::new()
        .prefix("cort-cache-")
        .tempdir()
        .unwrap();
    let cache = cache_dir.path().to_path_buf();
    (dir, root, cache_dir, cache)
}

const CONTROLLER: &str = concat!(
    "package demo;\n",
    "public class OrderController {\n",
    "  private final OrderService service;\n",
    "  public OrderController(OrderService service) { this.service = service; }\n",
    "  public void submit(Order o) {\n",
    "    service.save(o);\n",
    "  }\n",
    "}\n",
);

/// The whole product sentence for Java: the caller set is enumerated and each edge says the line
/// it came from, in the form the gate resolved it.
#[test]
fn impact_finds_the_java_caller_and_says_which_line_to_check() {
    let (_p, cwd, _c, cache) = java_sandbox(&[
        ("src/OrderService.java", SERVICE),
        ("src/OrderController.java", CONTROLLER),
    ]);
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let p = payload(&run_cort(
        &["impact", "--symbol", "OrderService::save", "-f", "json"],
        &cwd,
        &cache,
    ));
    assert_eq!(p["dependent_count"], 1);
    let dep = &p["dependents"][0];
    assert_eq!(dep["symbol_name"], "OrderController::submit");
    assert_eq!(dep["call_form"], "receiver");
    assert_eq!(dep["call_site_line"], 6);
}

/// `this.log()` inside `OrderService::save`: the enclosing symbol's owner is checked, not guessed,
/// so the edge exists even though `this` carries no name a heuristic could match.
#[test]
fn a_this_call_binds_to_the_enclosing_owner() {
    let (_p, cwd, _c, cache) = java_sandbox(&[("src/OrderService.java", SERVICE)]);
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let p = payload(&run_cort(
        &["impact", "--symbol", "OrderService::log", "-f", "json"],
        &cwd,
        &cache,
    ));
    assert_eq!(p["dependent_count"], 1);
    assert_eq!(p["dependents"][0]["symbol_name"], "OrderService::save");
}

/// The gate only ever refuses: `obj.reload()` where two `reload`s exist attaches nothing, and
/// coverage names the site instead of letting the caller set look complete.
#[test]
fn an_ambiguous_receiver_is_refused_and_surfaced_as_a_gap() {
    let two = concat!(
        "package demo;\n",
        "public class A { public void reload() {} }\n",
        "public class B { public void reload() {} }\n",
        "public class Uses {\n",
        "  public void go(A a) { a.reload(); }\n",
        "}\n",
    );
    let (_p, cwd, _c, cache) = java_sandbox(&[("src/All.java", two)]);
    assert_eq!(run_cort(&["index"], &cwd, &cache).code, 0);
    let p = payload(&run_cort(
        &["impact", "--symbol", "A::reload", "-f", "json"],
        &cwd,
        &cache,
    ));
    assert_eq!(p["dependent_count"], 0);
    let cov = payload(&run_cort(
        &[
            "impact",
            "--symbol",
            "A::reload",
            "--coverage",
            "-f",
            "json",
        ],
        &cwd,
        &cache,
    ));
    let seeds = cov["coverage"]["seeds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let refused: Vec<&Value> = seeds
        .iter()
        .flat_map(|s| s["extracted_but_unresolved"].as_array().unwrap())
        .collect();
    assert!(
        refused.iter().any(|row| row["file_path"] == "src/All.java"
            && row["raw_target"] == "a.reload"
            && row["from_symbol"] == "Uses::go"),
        "the refused a.reload() must be disclosed as an unresolved extraction, got {cov}"
    );
    assert_eq!(
        seeds[0]["enumeration_may_be_incomplete"], true,
        "a named refusal is a real gap, not a clean bill of health"
    );
}
