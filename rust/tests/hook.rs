//! The rule is judged on real traffic, so the fixtures here are verbatim commands taken from the
//! session transcripts that carried the deployed skill (2026-08-31 20:10 onward). Inventing
//! plausible-looking greps would measure the rule against my imagination of agent behaviour, which
//! is exactly the error the demand re-measurement was written to correct.

use cort::hook::{
    evidence_in, judge, search_from_grep_fields, search_from_shell, shell_search_decline,
    suggests_impact_shape, Evidence, SilenceReason, Verdict,
};

#[test]
fn the_one_call_site_search_in_the_corpus_fires() {
    // The agent hand-rolled "drop the declaration line" with a second grep -- that is cort's
    // DECLARATION_KEYWORDS, rebuilt inline and worse. If the rule misses this, it has no purpose.
    let hit = suggests_impact_shape(
        "grep -rn 'rate_limit(' /home/yanggf/a/ft/crates/api/src/routes/*.rs | grep -v 'pub async fn'",
    )
    .expect("the call-site shape must fire");
    assert_eq!(hit.symbol, "rate_limit");
}

#[test]
fn orientation_searches_from_the_same_corpus_stay_with_rg() {
    // Alternations are text hunts: several names OR'd together is not one symbol.
    assert!(suggests_impact_shape(
        r"grep -n 'thinking\|MAX_TOKENS\|max_tokens\|LLM_TIMEOUT_S\|model' crates/cct2/src/llm.rs"
    )
    .is_none());
    // Transcripts, logs and state files are not project source.
    assert!(
        suggests_impact_shape("grep -a 'cct2' ~/.nullclaw/skill-traces.jsonl | tail -4").is_none()
    );
    assert!(
        suggests_impact_shape("rg -l 'muse-spark' ~/.claude/projects --glob '*.jsonl' 2>/dev/null")
            .is_none(),
        "a hyphenated string in a transcript tree is not a symbol"
    );
    assert!(suggests_impact_shape(
        "grep -inE 'quota|429|rate.?limit|credit|exhaust' /home/yanggf/.claude/"
    )
    .is_none());
    // CJK prose.
    assert!(
        suggests_impact_shape(r"grep -rn '雙模型\|單模型\|no text block' crates/cct2/src/")
            .is_none()
    );
}

#[test]
fn the_shape_of_the_symbol_is_what_decides_not_the_tool() {
    // Qualified Rust methods are the form cort asks for.
    assert_eq!(
        suggests_impact_shape("rg 'Tally::add' rust/src")
            .unwrap()
            .symbol,
        "Tally::add"
    );
    // No path means the working tree, which in an agent session is the project.
    assert_eq!(
        suggests_impact_shape("rg resolve_targets").unwrap().symbol,
        "resolve_targets"
    );
    // Short names are noise on any corpus.
    assert!(suggests_impact_shape("rg fs src/").is_none());
    // A flag that takes a value must not be read as the pattern.
    assert_eq!(
        suggests_impact_shape("rg --glob '*.rs' receiver_binds src/")
            .unwrap()
            .symbol,
        "receiver_binds"
    );
    // `-e` names the pattern explicitly.
    assert_eq!(
        suggests_impact_shape("grep -rn -e cause_of rust/src/coverage.rs")
            .unwrap()
            .symbol,
        "cause_of"
    );
    // Not a search at all.
    assert!(suggests_impact_shape("cargo test --locked").is_none());
    assert!(suggests_impact_shape("sed -n '1,20p' src/lib.rs").is_none());
}

#[test]
fn the_three_shapes_the_first_probe_run_got_wrong() {
    // 1. A language cort does not index. `~/nullclaw/src/cron.zig` matched on the `src/` marker,
    //    but there is no Zig rule pack, so `impact` has nothing to say about it. Three of the 31.
    assert!(
        suggests_impact_shape(r#"grep -n "timeout" ~/nullclaw/src/cron.zig | head -40"#).is_none()
    );
    assert!(suggests_impact_shape(
        r#"grep -rn "reloadJobs(" ~/nullclaw/src/*.zig ~/nullclaw/src/**/*.zig 2>/dev/null"#
    )
    .is_none());

    // 2. Config and manifests are not source. This fired through the `crates/` marker.
    assert!(
        suggests_impact_shape("grep name backend-rust/crates/core/Cargo.toml | head -1").is_none()
    );

    // 3. Context flags mean the agent is reading the code, not enumerating its callers. Whoever
    //    asks for 10 lines around a match wants the body, and `cort context` is that verb.
    assert!(suggests_impact_shape(
        r#"grep -n -B2 -A10 "getVisitorAnalytics" backend/src/services/analyticsService.ts"#
    )
    .is_none());
    assert!(suggests_impact_shape(
        r#"grep -n -A 18 "validate" crates/worker/src/auth_handlers.rs"#
    )
    .is_none());

    // Still fires: the shapes adjudicated as genuine caller-set work in the same run.
    assert_eq!(
        suggests_impact_shape(
            "grep -rn 'deliver_news' crates/news/src/*.rs | grep -v '^crates/news/src/deliver.rs'"
        )
        .unwrap()
        .symbol,
        "deliver_news"
    );
    assert_eq!(
        suggests_impact_shape(
            r#"grep -rn "ensureSeedUserPasswords" backend/src frontend/src 2>/dev/null; echo "---(empty = fully removed)---""#
        )
        .unwrap()
        .symbol,
        "ensureSeedUserPasswords",
        "verifying a deletion is complete is the exact task the goal sentence names"
    );
    assert_eq!(
        suggests_impact_shape(
            r#"grep -rn "updatePaymentStatus" frontend/src --include="*.tsx" | grep -v "registrationService""#
        )
        .unwrap()
        .symbol,
        "updatePaymentStatus"
    );
}

#[test]
fn a_search_inside_one_named_file_is_reading_not_enumerating() {
    assert!(
        suggests_impact_shape("grep -n 'confidence' crates/cct2/src/merge.rs | head -30").is_none()
    );
    assert!(suggests_impact_shape(
        r#"grep -n "AI_SUBSTAGE_CACHE_VARIANT" crates/news/src/summarize.rs | head -2"#
    )
    .is_none());
    assert!(
        suggests_impact_shape(r#"grep -n "sales" crates/worker/src/lib.rs | head -20"#).is_none()
    );

    // Cross-file shapes survive: a directory, a glob, or a recursive flag.
    assert_eq!(
        suggests_impact_shape(
            r#"grep -rn "ensureSeedUserPasswords" backend/src frontend/src 2>/dev/null"#
        )
        .unwrap()
        .symbol,
        "ensureSeedUserPasswords"
    );
    assert_eq!(
        suggests_impact_shape("grep -rn 'SkillStatus::Degraded' crates/*/src/*.rs | sort -u")
            .unwrap()
            .symbol,
        "SkillStatus::Degraded"
    );
    assert_eq!(
        suggests_impact_shape(
            "grep -rn 'rate_limit(' /home/yanggf/a/ft/crates/api/src/routes/*.rs | grep -v 'pub async fn'"
        )
        .unwrap()
        .symbol,
        "rate_limit"
    );
    // Two named files is still a scoped read, not an enumeration.
    assert!(suggests_impact_shape(
        r#"grep -n "DUCKDB_PATH" backend/src/database/duckdb-connection.ts backend/src/database/duckdb-pool.ts"#
    )
    .is_none());
}

#[test]
fn a_redirection_is_not_a_search_target() {
    assert!(suggests_impact_shape(
        r#"grep -n "DUCKDB_PATH" backend/src/database/duckdb-connection.ts backend/src/database/duckdb-pool.ts 2>/dev/null"#
    )
    .is_none());
    assert_eq!(
        suggests_impact_shape(r#"grep -rn "ensureSeedUserPasswords" backend/src 2>/dev/null"#)
            .unwrap()
            .symbol,
        "ensureSeedUserPasswords",
        "stripping redirections must not strip the real directory beside them"
    );
}

/// The hook end of the rule: what the harness actually receives. Exercised through the built
/// binary, because the contract being tested is "what lands on stdout", not a function's return.
mod hook_command {
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn run(payload: &str, cache: &std::path::Path, cwd: &std::path::Path) -> String {
        let bin = env!("CARGO_BIN_EXE_cort");
        let mut child = Command::new(bin)
            .arg("hook-suggest")
            .current_dir(cwd)
            .env("CORT_CACHE_DIR", cache)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("cort runs");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("write");
        let out = child.wait_with_output().expect("wait");
        assert!(
            out.status.success(),
            "the hook must never fail the tool call"
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// An unindexed project is the common case on a machine with many checkouts. Suggesting a query
    /// that can only answer `no_seed_resolved` spends the agent's turn to say nothing, and the first
    /// time it happens the suggestion stops being worth reading.
    #[test]
    fn an_unindexed_project_gets_no_suggestion_even_on_a_matching_search() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cache = tmp.path().join("cache");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).expect("mkdir");
        let out = run(
            r#"{"tool_name":"Bash","tool_input":{"command":"grep -rn deliver_news crates/news/src/*.rs"}}"#,
            &cache,
            &project,
        );
        assert!(!out.contains("additionalContext"), "{out}");
    }

    #[test]
    fn a_payload_that_is_not_a_search_is_silent_and_still_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        for payload in [
            r#"{"tool_name":"Bash","tool_input":{"command":"cargo test --locked"}}"#,
            r#"{"tool_name":"Bash","tool_input":{}}"#,
            "not json at all",
            "",
        ] {
            let out = run(payload, tmp.path(), tmp.path());
            assert!(!out.contains("additionalContext"), "{payload} -> {out}");
        }
    }
}

// --- Structured searches ------------------------------------------------------------------------
//
// Same discipline as above: every fixture is a verbatim `Grep` tool call taken from
// ~/.kimi-code/sessions/**/wire.jsonl, so Kimi's parser is judged against traffic that happened
// rather than against a guess at what a structured search looks like. What these pin is that a
// second *parser* still reaches the one shared `judge` -- and reaches the same verdict its shell
// twin does.

fn grep_tool(pattern: &str, path: Option<&str>) -> Option<cort::hook::HookHit> {
    match judge(
        &search_from_grep_fields(pattern, path, None, false).expect("a non-empty pattern parses"),
        |_| Evidence::Unknown,
    ) {
        Verdict::Fire(hit) => Some(hit),
        Verdict::Silent(_) => None,
    }
}

#[test]
fn a_bare_symbol_grep_tool_call_fires_the_same_as_its_shell_twin() {
    // {"pattern":"enumeration_may_be_incomplete","path":"rust/src","output_mode":"content","-n":true}
    let hit = grep_tool("enumeration_may_be_incomplete", Some("rust/src"))
        .expect("a bare symbol scoped to source must fire");
    assert_eq!(hit.symbol, "enumeration_may_be_incomplete");
    assert_eq!(
        hit.symbol,
        suggests_impact_shape("rg -e 'enumeration_may_be_incomplete' rust/src")
            .unwrap()
            .symbol,
        "two parsers, one verdict -- that is the whole point of the split"
    );
}

/// The value on `-C` is exactly what a rendering back into a shell line had to throw away. Kimi's
/// parser keeps it as a flag rather than re-deriving it from text.
#[test]
fn a_context_flag_in_the_fields_silences_it_the_way_a_shell_flag_does() {
    // {"-C":4,"output_mode":"content","path":"frontend/src","pattern":"updatePaymentStatus"}
    let s = search_from_grep_fields("updatePaymentStatus", Some("frontend/src"), None, true)
        .expect("parses");
    assert!(
        matches!(judge(&s, |_| Evidence::Unknown), Verdict::Silent(_)),
        "a `-C` field is `cort context`'s question, not `impact`'s"
    );
    assert!(
        suggests_impact_shape("rg -C 4 -e 'updatePaymentStatus' frontend/src").is_none(),
        "and the shell twin must agree"
    );
}

#[test]
fn a_grep_tool_call_pinned_to_one_file_does_not_fire() {
    // {"-n":true,"output_mode":"content","path":"rust/src/main.rs","pattern":"cmd_hook_install"}
    // A source-language file, so this is stopped by the single-file gate rather than by the
    // extension gate -- which is the case a naive `recursive: true` would have got wrong.
    assert!(
        grep_tool("cmd_hook_install", Some("rust/src/main.rs")).is_none(),
        "one concrete file is reading, not enumerating a caller set"
    );
}

#[test]
fn an_alternation_pattern_is_not_a_symbol_in_either_form() {
    // {"pattern":"MAP_ORIGIN|MAP_STEP|MAP_COLS","path":"game/scripts", ...}
    assert!(grep_tool("MAP_ORIGIN|MAP_STEP|MAP_COLS", Some("game/scripts")).is_none());
}

/// `type` narrows which languages are read, not where, so it is not a target -- folding it in would
/// make a tree-wide search read as if it named a concrete path and stop firing.
#[test]
fn a_type_filtered_tree_wide_call_still_fires() {
    // {"pattern":"custom_topic_raw_listing","type":"rust"}
    let hit = grep_tool("custom_topic_raw_listing", None).expect("scoped to the working tree");
    assert_eq!(hit.symbol, "custom_topic_raw_listing");
}

/// A `glob` is a target: it is what makes a search cross-file, which is the shape `impact` answers.
#[test]
fn a_glob_field_counts_as_a_cross_file_target() {
    let s = search_from_grep_fields("visitorTracking", None, Some("*.ts"), false).expect("parses");
    let Verdict::Fire(hit) = judge(&s, |_| Evidence::Unknown) else {
        panic!("a globbed structured search is the shape");
    };
    assert_eq!(hit.symbol, "visitorTracking");
}

/// No shell, so no quoting, so no class of pattern the parser has to refuse. The only `None` left
/// is the one that was never a search.
#[test]
fn the_only_unparseable_structured_call_is_an_empty_pattern() {
    assert!(search_from_grep_fields("", Some("rust/src"), None, false).is_none());
    assert!(search_from_grep_fields("it's a \"quote\"", None, None, false).is_some());
}

/// The predicate is "a seed OR a raw edge naming this symbol", and the second half is what keeps the
/// deletion case alive: `raw_edges` outlives the `chunks` row it pointed at, so a just-deleted symbol
/// still has evidence even though `impact` can no longer seed on it.
///
/// `Unknown` fires because a lookup that could not run is not a finding. `NoIndex` is a distinct
/// silence from `NoEvidence`: one is a missed opportunity, the other is a correct refusal, and
/// `tests/cli.rs` documents why that distinction has to survive into the usage row.
#[test]
fn the_verdict_names_which_silence_it_chose() {
    let s = search_from_shell("grep -rn 'ensureSeedUserPasswords' src/").expect("parses");

    for (ev, label) in [
        (Evidence::Seed, "a symbol impact can seed on"),
        (
            Evidence::RawOnly,
            "a deleted symbol a surviving caller still names",
        ),
        (
            Evidence::Unknown,
            "a lookup that could not run must not silence",
        ),
    ] {
        assert!(
            matches!(judge(&s, |_| ev), Verdict::Fire(_)),
            "{label}: expected Fire"
        );
    }
    assert_eq!(
        judge(&s, |_| Evidence::Neither),
        Verdict::Silent(SilenceReason::NoEvidence)
    );
    assert_eq!(
        judge(&s, |_| Evidence::NoIndex),
        Verdict::Silent(SilenceReason::NoIndex)
    );
}

/// The lookup must not run for a search the shape gate already rejects, and the lookup is what opens
/// the database and shells out to git. On this machine's corpus the shape gate turns down about 95%
/// of searches; the hook's whole budget is 5s and `git rev-parse` may take 400ms of it.
#[test]
fn the_evidence_lookup_is_not_consulted_when_the_shape_gate_rejects() {
    let s = search_from_shell("grep -rn -A 3 'helper' src/").expect("parses");
    let mut consulted = false;
    let v = judge(&s, |_| {
        consulted = true;
        Evidence::Seed
    });
    assert_eq!(v, Verdict::Silent(SilenceReason::NoShape("context_flag")));
    assert!(
        !consulted,
        "a shape rejection must not open a database or run git"
    );
}

fn indexed_project(
    files: &[(&str, &str)],
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    rusqlite::Connection,
    String,
    String,
) {
    let dir = tempfile::tempdir().unwrap();
    for (rel, body) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, body).unwrap();
    }
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let mut db = cort::db::open_db(":memory:").unwrap();
    cort::db::ensure_schema(&db).unwrap();
    let project_id = cort::db::project_id_for(root.to_str().unwrap());
    let bin = cort::ast_grep::resolve_ast_grep_bin().expect("ast-grep on PATH");
    cort::indexer::full_index(&mut db, &bin, &root).unwrap();
    (dir, root, db, project_id, bin)
}

/// The three states against a real index. The `RawOnly` case is built the way it happens in life --
/// index, delete the definition, re-index incrementally, which is what the PostToolUse hook does --
/// because a hand-inserted row would not prove that `raw_edges` survives that path.
///
/// The surviving caller uses a QUALIFIED path on purpose. A bare call leaves a raw target the
/// exact-match arm alone would find, so both `LIKE` arms -- the half covering `crate::m::f`, which
/// `graph.rs` documents as the common shape -- would go unexercised and could be deleted with every
/// test still green.
#[test]
fn evidence_reads_chunks_then_raw_edges() {
    let (_dir, root, mut db, project_id, bin) = indexed_project(&[
        (
            "src/gone.rs",
            "pub fn ensure_seed_user_passwords() -> u8 { 1 }\n",
        ),
        (
            "src/user.rs",
            "pub fn boot() -> u8 { crate::gone::ensure_seed_user_passwords() }\n",
        ),
    ]);
    assert_eq!(
        evidence_in(&db, &project_id, "ensure_seed_user_passwords").unwrap(),
        Evidence::Seed
    );
    assert_eq!(
        evidence_in(&db, &project_id, "no_such_name_anywhere").unwrap(),
        Evidence::Neither
    );

    std::fs::remove_file(root.join("src/gone.rs")).unwrap();
    cort::incremental::incremental_index(
        &mut db,
        &bin,
        &root,
        cort::incremental::RebuildPolicy::Allow,
    )
    .unwrap();

    assert_eq!(
        evidence_in(&db, &project_id, "ensure_seed_user_passwords").unwrap(),
        Evidence::RawOnly,
        "the definition is gone but the surviving qualified call still names it"
    );
}

/// A storage failure must be returned, not flattened into `Neither`. `Neither` silences the hook; an
/// unreadable database has to fire instead, which is the caller's job and it needs the error to do
/// it. Both queries must fail, or the test would pass for the wrong reason.
#[test]
fn a_storage_failure_is_returned_rather_than_read_as_absence() {
    let (_dir, _root, db, project_id, _bin) =
        indexed_project(&[("src/lib.rs", "pub fn helper() -> u8 { 1 }\n")]);
    db.execute_batch("DROP TABLE relationships; DROP TABLE chunks; DROP TABLE raw_edges")
        .unwrap();
    assert!(
        evidence_in(&db, &project_id, "helper").is_err(),
        "a missing table is an error, not an absent symbol"
    );
}

/// The `rel_type IN ('calls','references')` filter is load-bearing and nothing else pins it. A Rust
/// `use` produces an `imports` raw target carrying the full path -- `crate::gone::tide` -- whose leaf
/// matches the `LIKE '%:'` arm, so without the filter a bare `tide` search would fire on a project
/// that merely imports the name and never calls it. Delete the filter and every other test stays
/// green.
///
/// (A JS/TS `./tide` specifier does *not* leak: it ends `/tide`, which matches neither arm. That was
/// worth checking rather than assuming -- an earlier comment here asserted the opposite.)
#[test]
fn an_import_path_is_not_evidence_that_a_symbol_exists() {
    let (_dir, _root, db, project_id, _bin) = indexed_project(&[
        ("src/gone.rs", "pub fn tide() -> u8 { 1 }\n"),
        // Imports the name and never calls it, so `tide` appears in an `imports` raw target and in
        // no `calls` one. The definition lives in a file this fixture then leaves alone, so the
        // seed half is what has to be removed for the raw-edge half to be the thing under test.
        (
            "src/user.rs",
            "use crate::gone::tide;\npub fn boot() -> u8 { 2 }\n",
        ),
    ]);
    let imports: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM raw_edges
              WHERE project_id = ?1 AND rel_type = 'imports' AND raw_target LIKE '%tide'",
            rusqlite::params![project_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        imports, 1,
        "the fixture must actually produce the import edge it is testing against"
    );

    // With the definition still indexed this is `Seed`; the raw-edge half only decides once the
    // chunk is gone, which is the deletion shape. Drop the chunk directly rather than re-indexing:
    // the point here is the query, not the incremental path (which `evidence_reads_chunks_then_raw_edges`
    // covers).
    assert_eq!(
        evidence_in(&db, &project_id, "tide").unwrap(),
        Evidence::Seed
    );
    db.execute(
        "DELETE FROM chunks WHERE project_id = ?1 AND symbol_name = 'tide'",
        rusqlite::params![project_id],
    )
    .unwrap();

    assert_eq!(
        evidence_in(&db, &project_id, "tide").unwrap(),
        Evidence::Neither,
        "an import path names a module route, not a call site impact could enumerate"
    );
}

/// A no_shape bucket nobody can attribute cannot be tuned: over the 30d window ending 2026-09-06
/// it held 83% of hook-suggest rows. Each decline now names the rule that rejected it, and the
/// tags are stable identifiers the mining groups on (issue #3). Fixtures are corpus-shaped, not
/// invented: every command below mirrors a real decline class from the transcripts already
/// pinned by the tests above.
#[test]
fn each_shape_rejection_names_the_rule_that_declined_it() {
    // Alternations are text hunts, not one symbol -- the extraction fails first.
    let s =
        search_from_shell(r"grep -rn 'thinking\|MAX_TOKENS\|max_tokens' crates/cct2/src/llm.rs")
            .expect("parses");
    assert_eq!(
        judge(&s, |_| Evidence::Seed),
        Verdict::Silent(SilenceReason::NoShape("pattern_not_symbol"))
    );

    // -A means "read around the match" -- context work, never caller enumeration.
    let s = search_from_shell("grep -rn -A 3 'helper' src/").expect("parses");
    assert_eq!(
        judge(&s, |_| Evidence::Seed),
        Verdict::Silent(SilenceReason::NoShape("context_flag"))
    );

    // Dependencies are not the project's own call sites.
    let s = search_from_shell("grep -rn 'helper' node_modules/").expect("parses");
    assert_eq!(
        judge(&s, |_| Evidence::Seed),
        Verdict::Silent(SilenceReason::NoShape("non_source_target"))
    );

    // A language the rule pack never parsed cannot be answered; suggesting impact there is the
    // worst kind of suggestion -- it looks answerable.
    let s = search_from_shell("grep -rn 'init' src/main.zig").expect("parses");
    assert_eq!(
        judge(&s, |_| Evidence::Seed),
        Verdict::Silent(SilenceReason::NoShape("unindexed_extension"))
    );

    // One named file, no recursion: reading, not enumerating.
    let s = search_from_shell("grep -n 'helper' src/main.rs").expect("parses");
    assert_eq!(
        judge(&s, |_| Evidence::Seed),
        Verdict::Silent(SilenceReason::NoShape("concrete_file_read"))
    );
}

/// The funnel has to separate the hook's baseline from its tuning targets: most Bash traffic is
/// not a search at all and the rule is right to stay quiet about it. Counting that traffic as
/// `unparseable_command` would score every future rule against noise (18 of the first 21 tagged
/// rows on 2026-09-07 -- `cargo`, `git`, `echo` -- were exactly this).
#[test]
fn non_search_tools_are_baseline_not_unparseable() {
    assert_eq!(
        shell_search_decline("cargo test --locked --all-targets"),
        "not_a_search_tool"
    );
    assert_eq!(
        shell_search_decline("git push origin main"),
        "not_a_search_tool"
    );
    assert_eq!(
        shell_search_decline("CORT_CACHE_DIR=/tmp/x cort status"),
        "not_a_search_tool",
        "leading VAR= assignments are skipped, same as the parser"
    );
    assert_eq!(
        shell_search_decline("grep --colour 'x' src/"),
        "unparseable_command",
        "a search tool whose pattern still fails to parse is the residual worth mining"
    );
    assert_eq!(
        shell_search_decline("/usr/bin/rg -n 'y'"),
        "unparseable_command",
        "absolute paths resolve to the same tool list"
    );
}
