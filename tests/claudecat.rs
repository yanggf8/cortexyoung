use std::fs;

#[test]
fn manifest_detects_rust_and_deps() {
    let dir = temp_project();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\n[dependencies]\nserde = \"1\"\nclap = \"4\"\n",
    )
    .unwrap();
    let (meta, deps) = claudecat::manifest::detect_project_meta(&dir);
    assert_eq!(meta.language, "Rust");
    assert!(deps
        .iter()
        .any(|g| g.ecosystem == "crates.io" && g.deps.contains(&"serde".to_string())));
}

#[test]
fn symbols_extract_rust_items() {
    let src = "pub struct User { name: String }\nimpl User { fn greet(&self) {} }\npub fn main() {}\nmod foo;\n";
    let syms = claudecat::symbols::extract_symbols("rust", src);
    let names: Vec<String> = syms.iter().map(|s| s.name.clone()).collect();
    assert!(names.iter().any(|n| n == "User"));
    assert!(names.iter().any(|n| n == "main"));
    assert!(syms.iter().any(|s| s.name.starts_with("impl")));
}

#[test]
fn symbols_skip_nested_fn_noise() {
    let src =
        "function outer() {\n  const tmp = 1;\n  function inner() {}\n}\nclass A { method() {} }\n";
    let syms = claudecat::symbols::extract_symbols("javascript", src);
    // nested const / inner fn must not be reported; class A + method() should
    assert!(!syms.iter().any(|s| s.name == "tmp"));
    assert!(!syms.iter().any(|s| s.name == "inner"));
    assert!(syms.iter().any(|s| s.name == "A"));
    assert!(syms.iter().any(|s| s.name == "method"));
}

#[test]
fn navigate_returns_document_ranges_without_turning_headings_into_symbols() {
    let map = claudecat::model::ProjectMap {
        document_headings: vec![claudecat::model::DocumentHeading {
            path: "README.md".into(),
            heading_path: vec!["Documented limitations".into()],
            start_line: 10,
            end_line: 22,
            preview: "coverage is a recall screen".into(),
        }],
        ..Default::default()
    };
    let result = claudecat::navigate::navigate(&map, "coverage");
    assert!(result.symbols.is_empty());
    assert_eq!(result.documents[0].start_line, 10);
    assert!(result
        .route
        .iter()
        .any(|line| line.contains("cort read README.md")));
}

#[test]
fn cort_navigation_keeps_document_route_when_no_symbol_matches() {
    let map = claudecat::model::ProjectMap {
        root: ".".into(),
        document_headings: vec![claudecat::model::DocumentHeading {
            path: "README.md".into(),
            heading_path: vec!["Completeness".into()],
            start_line: 3,
            end_line: 9,
            preview: "coverage is a recall screen".into(),
        }],
        ..Default::default()
    };
    let result = claudecat::navigate::navigate_with_cort(&map, "completeness", vec![], false);
    assert!(result.symbols.is_empty());
    assert!(result
        .route
        .iter()
        .any(|line| line.contains("cort read README.md")));
}

#[test]
fn navigation_usage_is_local_and_does_not_store_raw_query() {
    let _lock = cort_env_lock();
    let data = temp_project();
    let _env = EnvVarGuard(
        "CLAUDECAT_DATA_DIR".into(),
        std::env::var("CLAUDECAT_DATA_DIR").ok(),
    );
    std::env::set_var("CLAUDECAT_DATA_DIR", &data);
    let map = claudecat::model::ProjectMap {
        document_headings: vec![claudecat::model::DocumentHeading {
            path: "README.md".into(),
            heading_path: vec!["Coverage".into()],
            start_line: 2,
            end_line: 8,
            preview: "a recall screen".into(),
        }],
        ..Default::default()
    };
    let result = claudecat::navigate::navigate(&map, "private navigation phrase");
    claudecat::usage::record_navigation(
        std::path::Path::new("/tmp/demo-project"),
        "private navigation phrase",
        true,
        1,
        &result,
    );
    let report = claudecat::usage::report(30).unwrap();
    assert_eq!(report.events, 1);
    assert_eq!(report.cort_events, 1);
    let raw = std::fs::read(data.join("usage.db")).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("private navigation phrase"));
}

#[test]
fn claude_md_update_is_idempotent_and_atomic() {
    let dir = temp_project();
    let path = dir.join("CLAUDE.md");
    fs::write(&path, "# My Project\n\nsome content\n").unwrap();
    let (changed, content) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(changed);
    let (changed2, _) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(!changed2, "second update must be a no-op");
    assert!(
        !content.contains("<!-- claudecat:auto:begin -->"),
        "v2.1 起地圖不進 CLAUDE.md：auto 區塊不得再出現"
    );
    assert!(content.contains("# My Project"));
    assert!(content.contains("claudecat:guardrails:begin"));
    assert!(content.contains("claudecat:map-pointer:begin"));
    // no temp leftovers
    assert!(fs::read_dir(dir)
        .unwrap()
        .all(|e| e.unwrap().file_name() != ".claudecat.tmp"));
}

#[test]
fn strip_auto_block_removes_legacy_block_whole_lines() {
    let input = "# Rules\n\nbody text\n\n`<!-- claudecat:auto:begin -->\n## Project Map\n- **Root**: `/Users/guofang.mis/a/x`\n<!-- claudecat:auto:end -->\n\n<!-- claudecat:guardrails:begin -->\n<!-- claudecat:guardrails:end -->\n";
    let (out, removed) = claudecat::claude_md::strip_auto_block(input);
    assert!(removed);
    assert!(!out.contains("claudecat:auto"));
    assert!(!out.contains('`'), "行首殘留字元（反引號）應隨整行消失");
    assert!(!out.contains("Root"), "區塊內的機器路徑應整段消失");
    assert!(out.contains("# Rules"));
    assert!(out.contains("body text"));
    assert!(out.contains("claudecat:guardrails:begin"));
    assert!(!out.contains("\n\n\n"), "剝除後不得留下連續空行");
    let (out2, removed2) = claudecat::claude_md::strip_auto_block(&out);
    assert!(!removed2);
    assert_eq!(out, out2, "剝除必須冪等");
}

#[test]
fn strip_auto_block_truncates_when_end_marker_missing() {
    let input = "# Head\n\nbody\n\n<!-- claudecat:auto:begin -->\n## Map\n- torn";
    let (out, removed) = claudecat::claude_md::strip_auto_block(input);
    assert!(removed);
    assert_eq!(out, "# Head\n\nbody\n");
    assert!(!out.contains("torn"));
}

#[test]
fn strip_auto_block_noop_without_block() {
    let input = "# Rules\n\n- only rules\n";
    let (out, removed) = claudecat::claude_md::strip_auto_block(input);
    assert!(!removed);
    assert_eq!(out, input);
    let (out2, removed2) = claudecat::claude_md::strip_auto_block("");
    assert!(!removed2);
    assert_eq!(out2, "");
}

/// 暫時覆寫環境變數，drop 時還原（避免污染其他並行測試）
/// 序列化所有會改 process-global env（CORT_CACHE_DIR）的測試，
/// 避免 cargo 平行測試互相踩環境變數。
fn cort_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

struct EnvVarGuard(String, Option<String>);
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.1 {
            Some(v) => std::env::set_var(&self.0, v),
            None => std::env::remove_var(&self.0),
        }
    }
}

fn temp_project() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("claudecat-test-{}", std::process::id()));
    let dir = base.join(format!("{}", rand_suffix()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // 測試平行起跑時 nanos 會撞位（macOS 時鐘粒度 > 1ns），兩個 fixture 就會共用
    // 同一個目錄、互相覆寫對方的 main.rs（症狀：auto_profile 兩個測試隨機互換著掛）。
    // 時鐘再疊一個進程內單調序號，迴避次序高低位元，保證進程內唯一。
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    nanos ^ seq.rotate_left(20)
}

#[test]
fn guardrails_preserved_and_seeded() {
    let dir = temp_project();
    let path = dir.join("CLAUDE.md");
    fs::write(&path, "# P\n").unwrap();
    let (_, content) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(content.contains("claudecat:guardrails:begin"));
    // user adds a decision inside the guardrail block
    let edited = content.replace(
        "<!-- 技術決策 / Guardrails：每行一條",
        "<!-- 技術決策 / Guardrails：每行一條\n- 2D tilemap + Macroquad（禁 Python/3D）",
    );
    fs::write(&path, &edited).unwrap();
    // second update: marker already exists -> must NOT be re-seeded/overwritten
    let (_, content2) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(content2.contains("2D tilemap + Macroquad"));
    assert_eq!(content2.matches("claudecat:guardrails:begin").count(), 1);
}

#[test]
fn map_pointer_seeded_once_and_never_rewritten() {
    let dir = temp_project();
    let path = dir.join("CLAUDE.md");
    fs::write(&path, "# P\n").unwrap();
    let (_, content) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(content.contains("claudecat:map-pointer:begin"));
    // user hand-edits inside the pointer block
    let edited = content.replace(
        "播種一次，之後永不改寫",
        "播種一次，之後永不改寫（手改註記）",
    );
    fs::write(&path, &edited).unwrap();
    // second update: pointer already exists -> must NOT be re-seeded/overwritten
    let (_, content2) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(
        content2.contains("（手改註記）"),
        "指標區內的使用者文字不得被覆寫"
    );
    assert_eq!(content2.matches("claudecat:map-pointer:begin").count(), 1);
}

#[test]
fn data_dir_honors_env_override_and_xdg_default() {
    let _lock = cort_env_lock();
    let home = temp_project();
    let xdg = home.join("xdg-data");
    let over = home.join("override");
    let _g1 = EnvVarGuard(
        "CLAUDECAT_DATA_DIR".to_string(),
        std::env::var("CLAUDECAT_DATA_DIR").ok(),
    );
    let _g2 = EnvVarGuard(
        "XDG_DATA_HOME".to_string(),
        std::env::var("XDG_DATA_HOME").ok(),
    );
    let _g3 = EnvVarGuard("HOME".to_string(), std::env::var("HOME").ok());
    std::env::set_var("CLAUDECAT_DATA_DIR", &over);
    assert_eq!(claudecat::data_dir::data_dir(), over, "覆寫優先");
    std::env::remove_var("CLAUDECAT_DATA_DIR");
    std::env::set_var("XDG_DATA_HOME", &xdg);
    assert_eq!(
        claudecat::data_dir::data_dir(),
        xdg.join("claudecat"),
        "XDG_DATA_HOME 次之"
    );
    std::env::remove_var("XDG_DATA_HOME");
    std::env::set_var("HOME", &home);
    assert_eq!(
        claudecat::data_dir::data_dir(),
        home.join(".local/share/claudecat"),
        "預設落在 HOME 下"
    );
}

#[test]
fn map_path_uses_cort_project_id() {
    let p = "/some/real/path";
    assert_eq!(
        claudecat::data_dir::map_path_for(p),
        claudecat::data_dir::data_dir()
            .join("projects")
            .join(claudecat::cort::project_id(p))
            .join("map.md")
    );
}

#[test]
fn map_file_body_wraps_section() {
    let section = "## Project Map (auto-maintained by claudecat)\n- **Root**: `/x`\n";
    let body = claudecat::data_dir::map_file_body(section);
    assert!(body.starts_with("# claudecat Project Map\n"));
    assert!(body.contains(section), "section 應原樣置入");
    assert!(!body.contains("claudecat:auto"));
    assert!(body.ends_with('\n'));
}

#[test]
fn guardrails_load_from_file() {
    let dir = temp_project();
    let path = dir.join("claudecat-guardrails.md");
    fs::write(&path, "# decisions\n- 插件一律裝在 Claude Code 內\n").unwrap();
    let items = claudecat::guardrails::load(&dir, "");
    assert!(items.iter().any(|i| i.contains("插件一律裝")));
}

#[test]
fn explore_report_has_savings_section() {
    let dir = temp_project();
    fs::write(dir.join("Cargo.toml"), "[package]\nname=\"demo\"\n").unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    // 足夠大的 fixture（>420 行 code），full map 才能穩定低於全讀成本
    let mut body = String::from("use std::collections::HashMap;\n\nfn main() {}\n");
    for i in 0..420 {
        body.push_str(&format!("pub fn worker_{i}() -> usize {{ {i} }}\n"));
    }
    fs::write(dir.join("src/main.rs"), &body).unwrap();
    let map = claudecat_lib_scan(&dir);
    let m = claudecat::explore::compute(&map);
    let report = claudecat::explore::render(&m);
    assert!(report.contains("Map vs full-read") || report.contains("Map overhead"));
    assert!(report.contains("覆蓋率"));
    assert!(m.read_tokens > 0 && m.map_tokens > 0);
    assert!(m.savings_pct > 0.0);
}

fn claudecat_lib_scan(root: &std::path::Path) -> claudecat::model::ProjectMap {
    let mut map = claudecat::walk::analyze_project(root, 10, None);
    let (meta, deps) = claudecat::manifest::detect_project_meta(root);
    map.meta = meta;
    map.deps = deps;
    // 與 main.rs 的 analyze() 一致：對 key files 抽 symbols
    for f in &mut map.key_files {
        if let Some(lang) = &f.language {
            if let Ok(src) = std::fs::read_to_string(root.join(&f.path)) {
                let l = match lang.as_str() {
                    "typescript" => "typescript",
                    "javascript" => "javascript",
                    "python" => "python",
                    "rust" => "rust",
                    "go" => "go",
                    "c" => "c",
                    "cpp" => "cpp",
                    _ => continue,
                };
                f.symbols = claudecat::symbols::extract_symbols(l, &src);
            }
        }
    }
    map.generated_at = "2026-09-05T00:00:00Z".into();
    map
}

#[test]
fn track_appends_and_updates_same_day_row() {
    let dir = temp_project();
    let target = dir.join("SESSION-EVIDENCE.md");
    fs::write(dir.join("Cargo.toml"), "[package]\nname=\"demo\"\n").unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    let map = claudecat_lib_scan(&dir);
    let m = claudecat::explore::compute(&map);
    let (changed, _) = claudecat::explore::track_append(&target, &m).unwrap();
    assert!(changed);
    let content1 = fs::read_to_string(&target).unwrap();
    assert!(content1.contains("## 長期指標 (claudecat explore)"));
    assert!(content1.contains(m.date.as_str()));
    // same-day second run: row replaced, not duplicated
    let (changed2, _) = claudecat::explore::track_append(&target, &m).unwrap();
    assert!(!changed2, "same date+project row should be idempotent");
    let content2 = fs::read_to_string(&target).unwrap();
    assert_eq!(content2.matches(m.date.as_str()).count(), 1);
}

#[test]
fn auto_profile_picks_mini_for_small_project() {
    let dir = temp_project();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname=\"tiny\"\n").unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    let map = claudecat_lib_scan(&dir);
    let profile = claudecat::model::resolve_profile(map.total_loc, map.total_files, None);
    assert!(
        profile.is_mini(),
        "small project should resolve to Mini, got {:?}",
        profile
    );
    let md = claudecat::outline::render_with_profile(&map, profile);
    assert!(md.contains("Mini"));
    assert!(
        !md.contains("Key files & symbols"),
        "mini must skip symbols"
    );
}

#[test]
fn auto_profile_picks_full_for_large_project() {
    let dir = temp_project();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname=\"big\"\n").unwrap();
    let mut body = String::from("fn main() {}\n");
    for i in 0..500 {
        body.push_str(&format!("pub fn f{i}() -> usize {{ {i} }}\n"));
    }
    fs::write(dir.join("src/main.rs"), &body).unwrap();
    let map = claudecat_lib_scan(&dir);
    let profile = claudecat::model::resolve_profile(map.total_loc, map.total_files, None);
    assert!(!profile.is_mini(), "large project should resolve to Full");
}

#[test]
fn track_update_handles_multiple_repos() {
    let dir = temp_project();
    let target = dir.join("METRICS.md");
    let r1 = dir.join("repo1");
    let r2 = dir.join("repo2");
    for r in [&r1, &r2] {
        fs::create_dir_all(r.join("src")).unwrap();
        fs::write(r.join("Cargo.toml"), "[package]\nname=\"r\"\n").unwrap();
        fs::write(r.join("src/main.rs"), "fn main() {}\n").unwrap();
    }
    let m1 = claudecat::explore::compute(&claudecat_lib_scan(&r1));
    let m2 = claudecat::explore::compute(&claudecat_lib_scan(&r2));
    let refs = vec![&m1, &m2];
    let (changed, _) = claudecat::explore::track_update(&target, &refs).unwrap();
    assert!(changed);
    let content = fs::read_to_string(&target).unwrap();
    assert_eq!(
        content.matches("| 日期").count(),
        1,
        "single header expected"
    );
    assert_eq!(content.matches(&format!("`{}`", r1.display())).count(), 1);
    assert_eq!(content.matches(&format!("`{}`", r2.display())).count(), 1);
}

#[test]
fn grok_regression_dir_loc_no_double_count() {
    let dir = temp_project();
    fs::create_dir_all(dir.join("src/sub")).unwrap();
    fs::write(dir.join("src/a.rs"), "fn a(){}\n").unwrap();
    fs::write(dir.join("src/sub/b.rs"), "fn b(){}\n").unwrap();
    let map = claudecat_lib_scan(&dir);
    // 每個子目錄 loc 加總 == total_loc（雙計會讓它大於）
    let sum: usize = map.dir_stats.values().map(|d| d.loc).sum();
    assert_eq!(sum, map.total_loc, "目錄 LOC 不得雙計");
    let src = map.dir_stats.get("src").unwrap();
    let sub = map.dir_stats.get("src/sub").unwrap();
    assert_eq!(src.loc + sub.loc, map.total_loc);
}

#[test]
fn grok_regression_track_keeps_sibling_repo() {
    let dir = temp_project();
    let target = dir.join("M.md");
    let foo = dir.join("foo");
    let foobar = dir.join("foo-bar");
    for r in [&foo, &foobar] {
        fs::create_dir_all(r.join("src")).unwrap();
        fs::write(r.join("src/main.rs"), "fn main(){}\n").unwrap();
    }
    let m1 = claudecat::explore::compute(&claudecat_lib_scan(&foo));
    let m2 = claudecat::explore::compute(&claudecat_lib_scan(&foobar));
    claudecat::explore::track_update(&target, &[&m1]).unwrap();
    claudecat::explore::track_update(&target, &[&m2]).unwrap();
    // 更新 foo 時不得刪掉 foo-bar
    let m1b = claudecat::explore::compute(&claudecat_lib_scan(&foo));
    claudecat::explore::track_update(&target, &[&m1b]).unwrap();
    let content = fs::read_to_string(&target).unwrap();
    assert!(content.contains("foo-bar"), "sibling repo row must survive");
    assert!(content.contains(&format!("`{}`", foo.display())));
}

#[test]
fn grok_regression_update_root_stays_local() {
    let dir = temp_project();
    let deep = dir.join("sub/deep");
    fs::create_dir_all(&deep).unwrap();
    fs::write(dir.join("CLAUDE.md"), "# parent\n").unwrap();
    let section = "x";
    let path = claudecat::claude_md::find_claude_md(&deep);
    // 直接 join 回傳 root/CLAUDE.md，而非向上找父專案
    assert_eq!(path, deep.join("CLAUDE.md"));
    let _ = section;
}

#[test]
fn navigate_finds_symbol_and_route() {
    let dir = temp_project();
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("Cargo.toml"), "[package]\nname=\"demo\"\n").unwrap();
    fs::write(
        dir.join("src/main.rs"),
        "mod auth;\nfn main() {}\npub fn authenticate(name: &str) -> bool { true }\npub fn create_user() -> usize { 1 }\n",
    )
    .unwrap();
    let map = claudecat_lib_scan(&dir);
    let r = claudecat::navigate::navigate(&map, "auth");
    assert!(
        r.symbols.iter().any(|h| h.name == "authenticate"),
        "should hit authenticate"
    );
    // mod auth 精確命中優先；路線應含 cort context <命中符號>
    assert!(
        r.route.iter().any(|s| s.contains("cort context")),
        "route should suggest cort context"
    );
    let report_auth = claudecat::navigate::render(&r);
    assert!(report_auth.contains("auth") || report_auth.contains("authenticate"));
    let report = claudecat::navigate::render(&r);
    assert!(report.contains("路線"));
}

#[test]
fn cort_project_id_matches_sha256() {
    let id = claudecat::cort::project_id("/home/yanggf/a/claudecat");
    // 以 python hashlib 驗證過：77bf9a9b6e40...
    assert_eq!(&id[..12], "77bf9a9b6e40");
    // 確定性
    assert_eq!(id, claudecat::cort::project_id("/home/yanggf/a/claudecat"));
}

#[test]
fn cort_index_info_none_when_db_missing() {
    let dir = temp_project(); // 隨機目錄 → 無 cort DB
    let info = claudecat::cort::index_info(&dir);
    assert!(info.is_none(), "db 不存在應回 None，而非錯誤");
}

#[test]
fn cort_search_symbols_none_when_db_missing() {
    let dir = temp_project();
    let hits = claudecat::cort::search_symbols(&dir, "auth");
    assert!(hits.is_none());
}

/// cort 索引被寫入端持 EXCLUSIVE lock（模擬 cort hook 正在索引/寫入）時，
/// 一般唯讀 query 會 BUSY；claudecat 必須自動退回 immutable=1 仍能唯讀讀到索引。
#[cfg(unix)]
#[test]
fn cort_readonly_fallback_when_writer_holds_exclusive_lock() {
    use std::os::unix::fs::PermissionsExt;

    // 1) project + cache dir（cache 內放 cort 風格的 WAL DB）
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let db_path = cache.join(format!("{pid}.db"));

    // 2) WAL mode + schema v4 相容表 + 資料（checkpoint 後資料在主檔，唯讀可直接讀）
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE projects (
               project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
               git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
             );
             CREATE TABLE chunks (
               chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL,
               symbol_name TEXT, chunk_type TEXT, start_line INTEGER NOT NULL,
               end_line INTEGER NOT NULL, content TEXT NOT NULL, language TEXT
             );
             CREATE TABLE relationships (
               source_chunk_id TEXT NOT NULL, target_chunk_id TEXT NOT NULL,
               rel_type TEXT NOT NULL, call_site_line INTEGER, confidence_score REAL NOT NULL
             );
             INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, {now_ms}, 'test-extractor');
             INSERT INTO chunks VALUES ('c1', '{pid}', 'src/lib.rs', 'alpha', 'function', 1, 3, 'pub fn alpha() {{}}', 'Rust');
             INSERT INTO chunks VALUES ('c2', '{pid}', 'src/lib.rs', 'beta',  'function', 5, 9, 'pub fn beta() {{}}',  'Rust');
             INSERT INTO relationships VALUES ('c2', 'c1', 'calls', 6, 1.0);"
        ))
        .unwrap();
        // journal_mode 會回傳 row，須用 query_row 而非 execute_batch
        let mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let _: (i64, i64, i64) = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
    }
    let _ = fs::remove_file(format!("{}{}", db_path.display(), ".db-shm"));
    let _ = fs::remove_file(format!("{}{}", db_path.display(), ".db-wal"));

    // 3) 寫入端持 EXCLUSIVE lock，直到測試結束才釋放
    let holder = rusqlite::Connection::open(&db_path).unwrap();
    holder
        .execute_batch("PRAGMA locking_mode=EXCLUSIVE;")
        .unwrap();
    holder
        .execute_batch(
            "BEGIN; INSERT INTO chunks VALUES ('c3', 'x', 'x', 'x', 'x', 1, 1, 'x', 'x'); COMMIT;",
        )
        .unwrap();

    // 4) 情境成立：一般唯讀可以開，但第一次 query 就 BUSY
    let normal =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open 本身應成功");
    let err = normal.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get::<_, i64>(0));
    assert!(
        err.is_err(),
        "EXCLUSIVE lock 下一般唯讀 query 必須 BUSY，才能證明 fallback 有必要"
    );

    // 5) claudecat 的 fallback 仍可唯讀讀到索引（不寫入）
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let info = claudecat::cort::index_info(&proj).expect("immutable fallback 應能開啟");
    assert_eq!(info.chunk_count, 2);
    assert_eq!(info.relationships_count, 1);
    let hits = claudecat::cort::search_symbols(&proj, "alpha").expect("fallback 應能搜尋");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol.as_deref(), Some("alpha"));

    // 6) 清理：釋放 lock、還原 env
    drop(holder);
    drop(guard);
    let _ = std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755));
}

/// FTS 全文 fallback：symbol_name 未命中但 content 命中（`cort recall` 對應）。
/// 用 external-content FTS5 建合成 DB 實測唯讀查詢。
#[test]
fn cort_fts_finds_content_only_match() {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let db_path = cache.join(format!("{pid}.db"));

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE projects (
           project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
           git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
         );
         CREATE TABLE chunks (
           chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL,
           symbol_name TEXT, chunk_type TEXT, start_line INTEGER NOT NULL,
           end_line INTEGER NOT NULL, content TEXT NOT NULL, language TEXT
         );
         CREATE VIRTUAL TABLE chunks_fts USING fts5(
           content, symbol_name, file_path,
           content=chunks, content_rowid=rowid, tokenize='unicode61'
         );
         INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, 0, 'test');
         INSERT INTO chunks VALUES ('c1', '{pid}', 'src/cort.rs', 'with_readonly', 'function', 71, 90, 'fn with_readonly uses the immutable fallback to open the db', 'Rust');
         INSERT INTO chunks VALUES ('c2', '{pid}', 'src/main.rs', 'parse_cli', 'function', 10, 30, 'fn parse_cli parses the argv arguments', 'Rust');
         INSERT INTO chunks_fts(rowid, content, symbol_name, file_path)
           SELECT rowid, content, symbol_name, file_path FROM chunks;"
    ))
    .unwrap();
    drop(conn);

    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    // symbol_name LIKE 找不到（immutable 只在 content）→ FTS 全文找得到
    assert!(
        claudecat::cort::search_symbols(&proj, "immutable").is_none(),
        "symbol LIKE 不應命中 content"
    );
    let hits = claudecat::cort::search_fts(&proj, "immutable").expect("FTS 應命中 content");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].symbol.as_deref(), Some("with_readonly"));
    assert!(hits[0]
        .content
        .as_deref()
        .unwrap_or("")
        .contains("immutable"));

    // 多詞 AND（`"immutable" AND "fallback"`）
    let two = claudecat::cort::search_fts(&proj, "immutable fallback").expect("AND 應命中");
    assert_eq!(two.len(), 1);
    // 無關詞 → None（誠實回退）
    assert!(claudecat::cort::search_fts(&proj, "zzz_nothing").is_none());

    drop(guard);
}

/// navigate_with_cort：content 摘要進路線（省一次 read）+ FTS 來源標示
#[test]
fn navigate_with_cort_includes_content_summary() {
    use claudecat::model::{FileInfo, ProjectMap, Symbol};

    let map = ProjectMap {
        root: "/tmp/claudecat-nav-test".to_string(),
        key_files: vec![FileInfo {
            path: "src/cort.rs".to_string(),
            language: Some("rust".to_string()),
            loc: 100,
            symbols: vec![Symbol {
                kind: "fn".to_string(),
                name: "with_readonly".to_string(),
                line: 71,
            }],
        }],
        ..Default::default()
    };
    let hit = claudecat::cort::CortHit {
        symbol: Some("with_readonly".to_string()),
        chunk_type: "function".to_string(),
        file: "src/cort.rs".to_string(),
        start_line: 71,
        end_line: 90,
        language: Some("Rust".to_string()),
        content: Some(
            "fn with_readonly uses the immutable fallback to open the database file read-only"
                .to_string(),
        ),
    };

    // FTS 來源：標示 FTS 全文命中 + content 摘要 + 仍給 cort context 深挖路徑
    let r = claudecat::navigate::navigate_with_cort(&map, "immutable", vec![hit.clone()], true);
    assert!(
        r.route.iter().any(|s| s.contains("FTS 全文命中")),
        "FTS 來源應標示，實際：{:?}",
        r.route
    );
    assert!(
        r.route
            .iter()
            .any(|s| s.contains("內文摘要") && s.contains("immutable fallback")),
        "路線應含 content 摘要"
    );
    assert!(
        r.route
            .iter()
            .any(|s| s.contains("cort context with_readonly")),
        "仍應含 cort context 深挖路徑"
    );

    // 一般命中（symbol LIKE）：標示全量索引命中、摘要同樣帶上
    let r2 = claudecat::navigate::navigate_with_cort(&map, "with_readonly", vec![hit], false);
    assert!(r2.route.iter().any(|s| s.contains("全量索引命中")));
    assert!(r2.route.iter().any(|s| s.contains("內文摘要")));
}

/// cort-audit：索引健康 + 覆蓋缺口（file_state 有、chunks 無）+ FTS 同步
#[test]
fn cort_audit_reports_health_and_coverage() {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let db_path = cache.join(format!("{pid}.db"));

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE projects (
               project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
               git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
             );
             CREATE TABLE chunks (
               chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL,
               symbol_name TEXT, chunk_type TEXT, start_line INTEGER NOT NULL,
               end_line INTEGER NOT NULL, content TEXT NOT NULL, language TEXT,
               chunk_source TEXT DEFAULT 'ast'
             );
             CREATE TABLE file_state (
               project_id TEXT NOT NULL, file_path TEXT NOT NULL, file_content_hash TEXT NOT NULL
             );
             CREATE TABLE relationships (
               source_chunk_id TEXT NOT NULL, target_chunk_id TEXT NOT NULL,
               rel_type TEXT NOT NULL, call_site_line INTEGER, confidence_score REAL NOT NULL
             );
             INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, {now_ms}, 'test');
             INSERT INTO chunks VALUES ('c1', '{pid}', 'src/lib.rs', 'alpha', 'function', 1, 3, 'pub fn alpha()', 'Rust', 'ast');
             INSERT INTO chunks VALUES ('c2', '{pid}', 'src/lib.rs', 'beta',  'function', 5, 9, 'pub fn beta()',  'Rust', 'unparsed');
             INSERT INTO file_state VALUES ('{pid}', 'src/lib.rs', 'h1');
             INSERT INTO file_state VALUES ('{pid}', 'legacy/test-x.js', 'h2');
             INSERT INTO relationships VALUES ('c1', 'c2', 'calls', 2, 1.0);"
        ))
        .unwrap();
    }

    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let a = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    assert!(a.fresh, "索引 0 天前應 fresh");
    assert_eq!(a.chunk_count, 2);
    assert_eq!(a.relationships_count, 1);
    assert_eq!(a.file_state_files, Some(2));
    assert_eq!(a.chunked_files, Some(1));
    assert_eq!(a.not_chunked_total, Some(1));
    assert_eq!(a.not_chunked_files, vec!["legacy/test-x.js".to_string()]);
    assert_eq!(a.files_with_unparsed_chunks, 1);
    // 無 chunks_fts 表 → docs/drift 皆「無法判讀」（None），不得偽稱 synced 或 0
    assert_eq!(a.fts_docs, None);
    assert_eq!(a.fts_drift, None);

    drop(guard);
}

/// cort-audit：用量窗口（usage.db command_log；window 外不計 + hook JSON 解析）
#[test]
fn cort_audit_usage_counts_window() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let db_path = cache.join("usage.db");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, project_id TEXT,
               command TEXT NOT NULL, args_summary TEXT NOT NULL,
               status TEXT NOT NULL, error_code TEXT,
               read_source TEXT, requested_content_mode TEXT, effective_content_mode TEXT,
               receipt_hit INTEGER, index_stale INTEGER,
               bytes_out INTEGER NOT NULL, saved_bytes INTEGER NOT NULL
             );
             CREATE TABLE _usage_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
             VALUES (?1, 'impact', '{}', 'ok', 0, 100, 90)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
             VALUES (?1, 'hook-suggest', '{\"hook\":\"hit\"}', 'ok', 0, 0, 0)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
             VALUES (?1, 'impact', '{}', 'error', 1, 100, 10)",
            [&now],
        )
        .unwrap();
        // 窗口外（40 天前）不計
        conn.execute(
            "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
             VALUES (?1, 'context', '{}', 'ok', 0, 100, 0)",
            [&(now - 40 * 24 * 3600 * 1000)],
        )
        .unwrap();
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    assert_eq!(u.window_days, 30);
    assert_eq!(u.total_commands, 3);
    assert_eq!(u.by_command.get("impact"), Some(&2));
    assert_eq!(u.by_command.get("context"), None);
    assert_eq!(u.suggest_outcomes.get("hit"), Some(&1));
    assert_eq!(u.errors, 1);
    assert_eq!(u.index_stale_queries, 1);
    assert_eq!(u.saved_bytes, 100);

    drop(guard);
}

/// cort-audit：--track 寫進長期指標表（同日重複不新增列）
#[test]
fn cort_audit_track_updates_table() {
    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 7,
        index: None,
        db_exists: false,
        usage: None,
        usage_7d: None,
    };
    let dir = temp_project();
    let f = dir.join("EVIDENCE.md");
    let (changed1, _) = claudecat::cort_audit::track_update(&f, &[&a]).unwrap();
    assert!(changed1);
    let content1 = fs::read_to_string(&f).unwrap();
    assert!(content1.contains("## 長期指標 (claudecat cort-audit)"));
    assert!(content1.contains("| 日期 | 專案 | host | fresh |"));
    assert!(content1.contains("core/7d"), "追蹤列應含 core 動詞欄");
    assert!(content1.contains("deep/7d"), "追蹤列應含 deep 動詞欄");
    assert!(content1.contains("命令數/7d"), "追蹤列應含 7 天早期訊號欄");
    assert!(content1.contains("`/tmp/fake-root`"));

    let (changed2, _) = claudecat::cort_audit::track_update(&f, &[&a]).unwrap();
    assert!(!changed2, "同日同 root 重複 track 應是 no-op");
    let content2 = fs::read_to_string(&f).unwrap();
    assert_eq!(content1, content2);
}

/// P1 回歸：last_indexed_at 是「毫秒」（與 cort 寫入一致）——40 天前的索引必須 STALE，
/// 且 index_info（cort-status）與 audit_index（cort-audit）兩套判定口徑一致。
#[test]
fn cort_freshness_ms_stale_after_7_days_and_consistent() {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let db_path = cache.join(format!("{pid}.db"));
    let old_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 40 * 24 * 3600 * 1000;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE projects (
               project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
               git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
             );
             CREATE TABLE chunks (
               chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL
             );
             CREATE TABLE relationships (
               source_chunk_id TEXT NOT NULL, target_chunk_id TEXT NOT NULL,
               rel_type TEXT NOT NULL, call_site_line INTEGER, confidence_score REAL NOT NULL
             );
             INSERT INTO projects VALUES ('{pid}', 'old', '{real_str}', NULL, {old_ms}, 'test');"
        ))
        .unwrap();
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let info = claudecat::cort::index_info(&proj).expect("index_info 應有結果");
    assert!(!info.fresh, "40 天前的索引（毫秒），cort-status 應報 STALE");
    let audit = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    assert!(!audit.fresh, "audit 的 fresh 判定口徑應與 index_info 一致");
    drop(guard);
}

/// P1 回歸：--track 表格之後的內容（使用者筆記）必須原樣保留，
/// 不得被「section 掃到 EOF」的重寫刪掉。
#[test]
fn cort_audit_track_preserves_content_after_section() {
    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 7,
        index: None,
        db_exists: false,
        usage: None,
        usage_7d: None,
    };
    let dir = temp_project();
    let f = dir.join("EVIDENCE.md");
    let (c1, _) = claudecat::cort_audit::track_update(&f, &[&a]).unwrap();
    assert!(c1);
    // 使用者緊接在表格後加自己的筆記（無空行，讓同日重跑可精確 no-op）
    let mut content = fs::read_to_string(&f).unwrap();
    content.push_str("## 我的筆記\n\n重要結論 KEEPME-123\n");
    fs::write(&f, &content).unwrap();

    let (changed, _) = claudecat::cort_audit::track_update(&f, &[&a]).unwrap();
    assert!(!changed, "同日同 root、內容不變 → 應是 no-op");
    let after = fs::read_to_string(&f).unwrap();
    assert!(after.contains("KEEPME-123"), "表格後的筆記不應被刪除");
    assert!(after.contains("## 我的筆記"), "筆記 section 標題應保留");
}

/// P1 回歸：同一檔案裡 audit section 之後還有 explore section 時，
/// 更新 audit 不得刪除 explore section、也不得把它的資料列吞進 audit 表。
#[test]
fn track_table_preserves_sibling_sections_in_one_file() {
    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 7,
        index: None,
        db_exists: false,
        usage: None,
        usage_7d: None,
    };
    let dir = temp_project();
    let f = dir.join("SESSION-EVIDENCE.md");
    // 1) 先建立 audit 表格
    claudecat::cort_audit::track_update(&f, &[&a]).unwrap();
    // 2) 之後接一段 explore section（兩種指標共用同一份證據檔）
    let mut content = fs::read_to_string(&f).unwrap();
    content.push_str(
        "\n## 長期指標 (claudecat explore)\n\n| 日期 | 專案 |\n|---|---|\n| 2026-09-01 | `/old` |\n",
    );
    fs::write(&f, &content).unwrap();

    // 3) 同日再更新 audit（audit 表格本身等價重建；重點在下方內容斷言）
    let (changed, _) = claudecat::cort_audit::track_update(&f, &[&a]).unwrap();
    assert!(
        changed,
        "重寫會壓掉表格與下一個 section 間的空行 → 內容有變"
    );
    let after = fs::read_to_string(&f).unwrap();
    assert!(
        after.contains("## 長期指標 (claudecat explore)"),
        "explore section 標題不應被刪除"
    );
    assert!(
        after.contains("| 2026-09-01 | `/old` |"),
        "explore 的資料列不應被吞進 audit 表或刪除"
    );
    assert_eq!(
        after.matches(claudecat::cort_audit::TRACK_SECTION).count(),
        1,
        "audit section 標題應恰有一個"
    );
    assert!(
        after.matches("`/tmp/fake-root`").count() == 1,
        "audit 資料列應恰有一列（同日更新不重複）"
    );
}

/// P1 回歸：CLAUDE.md 是 symlink 時（cortexyoung 慣例：CLAUDE.md -> AGENTS.md，
/// 讓 Claude/Codex 兩個 harness 永不漂移），update 必須寫進 symlink 目標、
/// 不得把 symlink 取代成普通檔。v2.1 起「寫進目標」的內容是剝除舊版 auto 區塊
/// ＋播種——帶 legacy 區塊的本體經一次 update 應變成 rules-only。
#[test]
fn claude_md_update_writes_through_symlink() {
    let dir = temp_project();
    let legacy = "# AGENTS\n\nbody\n\n`<!-- claudecat:auto:begin -->\n## Project Map (auto-maintained by claudecat)\n- **Root**: `/home/yanggf/a/claudecat`\n<!-- claudecat:auto:end -->\n";
    fs::write(dir.join("AGENTS.md"), legacy).unwrap();
    std::os::unix::fs::symlink("AGENTS.md", dir.join("CLAUDE.md")).unwrap();

    let path = dir.join("CLAUDE.md");
    let (changed, _) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(changed);

    let meta = path.symlink_metadata().unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "symlink 不得被原子寫入取代成普通檔"
    );
    let agents = fs::read_to_string(dir.join("AGENTS.md")).unwrap();
    assert!(agents.contains("# AGENTS"), "原內容保留");
    assert!(
        !agents.contains("claudecat:auto"),
        "舊版 auto 區塊應經 symlink 從本體剝除"
    );
    assert!(!agents.contains("/home/yanggf"), "機器路徑隨區塊消失");
    assert!(
        agents.contains("claudecat:map-pointer:begin"),
        "指標種子應播下"
    );

    let (changed2, _) = claudecat::claude_md::update_section(&path, false).unwrap();
    assert!(!changed2, "同內容重跑應 no-op");
}

/// P2 回歸：file_state 表不存在（cort schema 版本差異）時，覆蓋必須是「無法判讀」，
/// 不得被 unwrap_or(0) 吞成「無缺口」——與當初 ?1 bug 同類的靜默零值。
#[test]
fn cort_audit_missing_file_state_table_is_not_silent_zero() {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let db_path = cache.join(format!("{pid}.db"));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // 故意不建 file_state / chunks_fts 表
        conn.execute_batch(&format!(
            "CREATE TABLE projects (
               project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
               git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
             );
             CREATE TABLE chunks (
               chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL
             );
             INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, {now_ms}, 'test');"
        ))
        .unwrap();
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let a = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    assert_eq!(
        a.not_chunked_total, None,
        "查詢失敗必須是「無法判讀」，不是 0"
    );
    assert_eq!(a.file_state_files, None);

    let audit = claudecat::cort::CortAudit {
        root: real_str,
        host: "test-host".to_string(),
        window_days: 30,
        index: Some(a),
        db_exists: true,
        usage: None,
        usage_7d: None,
    };
    let report = claudecat::cort_audit::render(&audit);
    assert!(
        report.contains("無法判讀"),
        "報告應明說無法判讀，不假裝無缺口"
    );
    drop(guard);
}

/// P2 回歸：FTS 同步判定用 rowid 雙向差異（drift），不用數量相等——
/// 一多一少時「數量相等」會偽稱 synced。
#[test]
fn cort_audit_fts_drift_detects_missing_and_extra_fts_rows() {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let db_path = cache.join(format!("{pid}.db"));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // drift 查詢只用 rowid join——測試用普通表即可，不需 fts5 模組
        conn.execute_batch(&format!(
            "CREATE TABLE projects (
               project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
               git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
             );
             CREATE TABLE chunks (
               chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL
             );
             CREATE TABLE chunks_fts (content TEXT);
             INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, {now_ms}, 'test');
             INSERT INTO chunks VALUES ('c1', '{pid}', 'a.rs');
             INSERT INTO chunks VALUES ('c2', '{pid}', 'b.rs');
             INSERT INTO chunks VALUES ('c3', '{pid}', 'c.rs');
             INSERT INTO chunks_fts (rowid, content) VALUES (1, 'x');
             INSERT INTO chunks_fts (rowid, content) VALUES (99, 'ghost');"
        ))
        .unwrap();
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let a = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    // docs 數量相等（2 == 2）但內容錯位：c2/c3 沒進 FTS、rowid 99 是孤兒
    assert_eq!(a.fts_docs, Some(2));
    assert_eq!(a.chunk_count, 3);
    assert_eq!(a.fts_drift, Some(3), "缺 2（c2,c3）+ 孤兒 1（rowid 99）= 3");
    assert_ne!(a.fts_drift, Some(0), "數量相等不得判為 synced");
    drop(guard);
}

/// P3 回歸：immutable URI 的路徑必須 percent-encode `%` `?` `#` 空白，
/// 否則 URI 語法會把路徑截斷在 query/fragment 邊界。
#[test]
fn cort_sqlite_uri_escapes_special_chars() {
    let p = std::path::Path::new("/home/u#1/my cache/db?x.db");
    let uri = claudecat::cort::sqlite_uri(p);
    assert_eq!(
        uri, "file:/home/u%231/my%20cache/db%3Fx.db?immutable=1",
        "URI 邊界字元必須編碼"
    );
    assert!(!uri.contains(' '));
}

/// P3 回歸：DB 檔存在但讀取失敗（如非 SQLite 檔、schema 全不相容）時，
/// 必須回報「存在但無法讀取」，不是誤報「尚未建立索引」。
#[test]
fn cort_audit_distinguishes_unreadable_db_from_missing_index() {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    // 檔案在，但不是 SQLite 資料庫
    fs::write(
        cache.join(format!("{pid}.db")),
        b"definitely not a database",
    )
    .unwrap();

    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    assert!(claudecat::cort::db_exists(&proj), "DB 檔存在");
    assert!(claudecat::cort::audit_index(&proj).is_none(), "讀取失敗");
    let a = claudecat::cort::audit(&proj, 30);
    assert!(a.db_exists);
    assert!(a.index.is_none());
    let report = claudecat::cort_audit::render(&a);
    assert!(
        report.contains("存在但無法讀取"),
        "應說「DB 存在但無法讀取」，不是「尚未建立索引」"
    );
    drop(guard);
}

/// P2 回歸：usage.db 是每台機器各自的——同一天、同 root、不同 host 的兩列
/// 必須並存，不得互相覆蓋（多機匯集到同一份 CORT-AUDIT.md 時）。
#[test]
fn cort_audit_track_rows_from_different_hosts_coexist() {
    let mk = |host: &str| claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: host.to_string(),
        window_days: 7,
        index: None,
        db_exists: false,
        usage: None,
        usage_7d: None,
    };
    let dir = temp_project();
    let f = dir.join("CORT-AUDIT.md");
    let (c1, _) = claudecat::cort_audit::track_update(&f, &[&mk("machine-a")]).unwrap();
    assert!(c1);
    let (c2, _) = claudecat::cort_audit::track_update(&f, &[&mk("machine-b")]).unwrap();
    assert!(c2, "不同 host 的新列應新增，不覆蓋 machine-a");
    let content = fs::read_to_string(&f).unwrap();
    assert_eq!(content.matches("/tmp/fake-root").count(), 2, "兩列並存");
    assert!(content.contains("machine-a") && content.contains("machine-b"));
    // 同 host 同日重跑 → 只更新自己那列
    let (_, _) = claudecat::cort_audit::track_update(&f, &[&mk("machine-a")]).unwrap();
    let content2 = fs::read_to_string(&f).unwrap();
    assert_eq!(
        content2.matches("/tmp/fake-root").count(),
        2,
        "同日同 host 重跑不增列"
    );
}

/// doctor：host 探測在這台機器上要拿得到名字——macOS 沒有 /etc/hostname，
/// 靠 `hostname` 指令退路；拿不到的機器在審計表裡永遠是 unknown 那一列。
#[test]
fn doctor_host_resolves_on_this_machine() {
    let host = claudecat::doctor::resolve_host()
        .expect("host probe returned None: neither /etc/hostname nor `hostname` answered");
    assert!(!host.is_empty());
    assert!(!host.contains('\n'), "trim 過的名字不該帶換行");
}

/// doctor：cron 條目的產生、偵測與幂等合併（部署一件事的純函式核心）
#[test]
fn doctor_track_cron_line_detection_and_merge_idempotent() {
    let line = claudecat::doctor::track_cron_line(
        "/home/u/claudecat",
        std::path::Path::new("/home/u/proj"),
    );
    assert!(line.starts_with("17 9 * * * "), "避開整點的分鐘數");
    assert!(line.contains("cort-audit --root /home/u/proj --track CORT-AUDIT.md"));
    assert!(line.contains("cort-audit.log"), "輸出要落 log 便于診斷");

    assert!(!claudecat::doctor::has_track_entry(""));
    assert!(claudecat::doctor::has_track_entry(&line));

    let once = claudecat::doctor::merge_entry("", &line, claudecat::doctor::has_track_entry);
    assert_eq!(once.lines().count(), 1);
    let twice = claudecat::doctor::merge_entry(&once, &line, claudecat::doctor::has_track_entry);
    assert_eq!(twice, once, "幂等：已安裝不得重複");

    let kept = claudecat::doctor::merge_entry(
        "0 0 * * * echo hi\n",
        &line,
        claudecat::doctor::has_track_entry,
    );
    assert!(
        kept.starts_with("0 0 * * * echo hi\n") && kept.lines().count() == 2,
        "既有條目必須保留"
    );

    // 09:29 分析條目：headless claude -p + repo 裡的 prompt 檔 + 落 log
    let aline = claudecat::doctor::analysis_cron_line(
        "/home/u/.local/bin/claude",
        "/home/u/claudecat",
        &[],
    );
    assert!(aline.starts_with("29 9 * * * "), "與 09:17 錯開");
    assert!(aline.contains("--dangerously-skip-permissions"));
    assert!(aline.contains("$(cat /home/u/claudecat/cort-audit-analysis-prompt.md)"));
    assert!(aline.contains("cort-audit-analysis.log"));
    assert!(claudecat::doctor::has_analysis_entry(&aline));
    assert!(
        !claudecat::doctor::has_analysis_entry(&line),
        "track 條目不得誤判為 analysis 條目"
    );
    let both = claudecat::doctor::merge_entry(&once, &aline, claudecat::doctor::has_analysis_entry);
    assert_eq!(both.lines().count(), 2, "兩條並存");
    let again =
        claudecat::doctor::merge_entry(&both, &aline, claudecat::doctor::has_analysis_entry);
    assert_eq!(again, both, "analysis 條目幂等");
}

/// 額度會枯竭，偏好就要能調頭：既有分析條目用的執行檔 ≠ 目前偏好 → 重寫；
/// 已是偏好 → 不動。使用者調回來 = 換 CLAUDECAT_ANALYSIS_BIN 重跑 --install。
#[test]
fn doctor_analysis_entry_swaps_runner_on_preference_change() {
    let old_line = claudecat::doctor::analysis_cron_line(
        "/home/u/.local/bin/claude",
        "/home/u/claudecat",
        &[],
    );
    let new_line = claudecat::doctor::analysis_cron_line(
        "/home/u/.local/bin/musecode",
        "/home/u/claudecat",
        &[],
    );
    let track = claudecat::doctor::track_cron_line(
        "/home/u/claudecat",
        std::path::Path::new("/home/u/proj"),
    );
    let existing = format!("{track}\n{old_line}");

    // 偏好換成 musecode → 舊 claude 條目被替換，track 保留
    let (out, changed) = claudecat::doctor::merge_analysis_entry(&existing, &new_line);
    assert!(changed);
    assert!(out.contains("musecode") && !out.contains("/home/u/.local/bin/claude -p"));
    assert!(out.contains("17 9 * * *"), "track 條目保留");

    // 已是偏好 → 不動（幂等）
    let (same, changed2) = claudecat::doctor::merge_analysis_entry(&out, &new_line);
    assert!(!changed2);
    assert_eq!(same, out);
}

/// CLAUDECAT_ANALYSIS_BIN 指名時必須被尊重（額度調頭的開關）。
#[test]
fn doctor_analysis_bin_honors_env_override() {
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CLAUDECAT_ANALYSIS_BIN".to_string(),
        std::env::var("CLAUDECAT_ANALYSIS_BIN").ok(),
    );
    std::env::set_var("CLAUDECAT_ANALYSIS_BIN", "no-such-analysis-binary-xyz");
    let err = claudecat::doctor::analysis_bin().unwrap_err();
    assert!(
        err.contains("no-such-analysis-binary-xyz"),
        "錯誤要指名找不到誰"
    );

    std::env::set_var("CLAUDECAT_ANALYSIS_BIN", "claude");
    let bin = claudecat::doctor::analysis_bin().expect("claude 在本機存在");
    assert!(bin.ends_with("claude"));
    drop(guard);
}

/// cort-audit：hook-suggest 的 decline 歸因進 UsageWindow（cortexyoung c290c383 起，
/// 新列才帶 decline；舊列與無 decline 的 outcome 自然缺席）+ 報告呈現 top declines。
#[test]
fn cort_audit_usage_tracks_decline_distribution() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let db_path = cache.join("usage.db");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, project_id TEXT,
               command TEXT NOT NULL, args_summary TEXT NOT NULL,
               status TEXT NOT NULL, error_code TEXT,
               read_source TEXT, requested_content_mode TEXT, effective_content_mode TEXT,
               receipt_hit INTEGER, index_stale INTEGER,
               bytes_out INTEGER NOT NULL, saved_bytes INTEGER NOT NULL
             );",
        )
        .unwrap();
        let ins = |conn: &rusqlite::Connection, summary: &str| {
            conn.execute(
                "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
                 VALUES (?1, 'hook-suggest', ?2, 'ok', 0, 0, 0)",
                rusqlite::params![now, summary],
            )
            .unwrap();
        };
        ins(
            &conn,
            r#"{"hook":"no_shape","v":3,"decline":"context_flag"}"#,
        );
        ins(
            &conn,
            r#"{"hook":"no_shape","v":3,"decline":"context_flag"}"#,
        );
        ins(
            &conn,
            r#"{"hook":"no_shape","v":3,"decline":"pattern_not_symbol"}"#,
        );
        // baseline（本來就不是搜尋的命令）比 context_flag 多，但不得贏走 decline-top
        ins(
            &conn,
            r#"{"hook":"no_shape","v":3,"decline":"not_a_search_tool"}"#,
        );
        ins(
            &conn,
            r#"{"hook":"no_shape","v":3,"decline":"not_a_search_tool"}"#,
        );
        ins(
            &conn,
            r#"{"hook":"no_shape","v":3,"decline":"not_a_search_tool"}"#,
        );
        ins(&conn, r#"{"hook":"hit","v":3}"#);
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    assert_eq!(
        u.suggest_outcomes.get("no_shape"),
        Some(&6),
        "2 context_flag + 1 pattern_not_symbol + 3 baseline"
    );
    assert_eq!(u.suggest_outcomes.get("hit"), Some(&1));
    assert_eq!(
        u.declines.get("no_shape/context_flag"),
        Some(&2),
        "同標籤要累計"
    );
    assert_eq!(u.declines.get("no_shape/pattern_not_symbol"), Some(&1));
    assert_eq!(u.declines.get("no_shape/not_a_search_tool"), Some(&3));
    assert_eq!(u.declines.len(), 3, "無 decline 的列不得進分佈");

    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    };
    let report = claudecat::cort_audit::render(&a);
    assert!(report.contains("top declines"), "報告應有 top declines 段");
    assert!(
        report.contains("no_shape/context_flag: 2"),
        "排序後最高者第一"
    );
    assert!(
        !report.contains("- no_shape/not_a_search_tool: 3"),
        "baseline 不進可動作排序（呈報行的 - （baseline 前綴除外）"
    );
    assert!(
        report.contains("baseline not_a_search_tool: 3"),
        "baseline 另行呈報，不隱藏"
    );
    let row = claudecat::cort_audit::row_md(&a);
    assert!(row.contains("context_flag=2"), "追蹤列應帶 decline-top");
    assert!(
        !row.contains("not_a_search_tool"),
        "baseline 不得佔 decline-top"
    );
    drop(guard);
}

/// `_cortex_meta` 三態 fixture：建一個「時戳全新、HEAD 不追究」的 cort 風格 DB，
/// `meta` 決定 `_cortex_meta` 的內容：None＝整張表不存在（舊版 cort DB）。
fn cort_meta_fixture(meta: Option<&[(&str, &str)]>) -> (std::path::PathBuf, std::path::PathBuf) {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-meta-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let conn = rusqlite::Connection::open(cache.join(format!("{pid}.db"))).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE projects (
           project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
           git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
         );
         CREATE TABLE chunks (
           chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL,
           symbol_name TEXT, chunk_type TEXT, start_line INTEGER NOT NULL,
           end_line INTEGER NOT NULL, content TEXT NOT NULL, language TEXT,
           chunk_source TEXT DEFAULT 'ast'
         );
         CREATE TABLE file_state (
           project_id TEXT NOT NULL, file_path TEXT NOT NULL, file_content_hash TEXT NOT NULL
         );
         CREATE TABLE relationships (
           source_chunk_id TEXT NOT NULL, target_chunk_id TEXT NOT NULL,
           rel_type TEXT NOT NULL, call_site_line INTEGER, confidence_score REAL NOT NULL
         );
         INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, {now_ms}, 'test');
         INSERT INTO chunks VALUES ('c1', '{pid}', 'src/lib.rs', 'alpha', 'function', 1, 3, 'pub fn alpha()', 'Rust', 'ast');
         INSERT INTO file_state VALUES ('{pid}', 'src/lib.rs', 'h1');"
    ))
    .unwrap();
    if let Some(rows) = meta {
        conn.execute_batch(
            "CREATE TABLE _cortex_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        for (k, v) in rows {
            conn.execute("INSERT INTO _cortex_meta VALUES (?1, ?2)", [k, v])
                .unwrap();
        }
    }
    drop(conn);
    (proj, cache)
}

fn audit_row(a: claudecat::cort::CortAuditIndex) -> String {
    claudecat::cort_audit::row_md(&claudecat::cort::CortAudit {
        root: a.path.clone(),
        host: "test-host".to_string(),
        window_days: 30,
        index: Some(a),
        db_exists: true,
        usage: None,
        usage_7d: None,
    })
}

/// P1 回歸：cort 的 schema 遷移會設 `graph_pending=1` 而**不動** `last_indexed_at`/`git_head`
/// （cort `db.rs:322` vs `incremental.rs` 的同一 transaction 清 0），
/// 所以「時戳很新但圖是舊的」是真實狀態——claudecat 不得報 fresh。
#[test]
fn cort_graph_pending_is_not_fresh_in_both_readers() {
    let (proj, cache) = cort_meta_fixture(Some(&[("SCHEMA_VERSION", "5"), ("graph_pending", "1")]));
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let info = claudecat::cort::index_info(&proj).expect("index_info 應有結果");
    assert_eq!(info.graph_pending, Some(true));
    assert_eq!(info.schema_version.as_deref(), Some("5"));
    assert!(!info.fresh, "圖已知落後時不得報 fresh（時戳再新也一樣）");

    let a = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    assert_eq!(a.graph_pending, Some(true));
    assert_eq!(
        a.fresh, info.fresh,
        "cort-status 與 cort-audit 必須同一口徑（09-06 毫秒事件的教訓）"
    );
    assert!(
        audit_row(a).contains("STALE/graph"),
        "日表要分得出「圖落後」與 HEAD/age 落後，且不加欄"
    );
    drop(guard);
}

/// `graph_pending=0`（每次增量在同一 transaction 清 0）→ 行為與過去完全一致。
#[test]
fn cort_graph_pending_zero_keeps_fresh() {
    let (proj, cache) = cort_meta_fixture(Some(&[("SCHEMA_VERSION", "5"), ("graph_pending", "0")]));
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let info = claudecat::cort::index_info(&proj).expect("index_info 應有結果");
    assert_eq!(info.graph_pending, Some(false));
    assert!(info.fresh, "圖已重建 + 時戳新 → 仍是 fresh");
    let a = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    assert_eq!(a.fresh, info.fresh);
    let row = audit_row(a);
    assert!(row.contains("| fresh |"), "日表值不變（跨日可比）");
    drop(guard);
}

/// 舊版 cort DB 沒有 `_cortex_meta`：讀不到 ≠ 壞掉，也 ≠ 健康。
/// 兩欄 None、`fresh` 仍只由 HEAD+age 決定（絕不因為讀不到就打成 STALE——
/// 那正是 cort 自己修掉的「沒讀到卻報 drifted」）。
#[test]
fn cort_missing_meta_table_is_unknown_not_unhealthy() {
    let (proj, cache) = cort_meta_fixture(None);
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let info = claudecat::cort::index_info(&proj).expect("index_info 應有結果");
    assert_eq!(
        info.graph_pending, None,
        "讀不到就是無法判讀，不得 Some(false)"
    );
    assert_eq!(info.schema_version, None);
    assert!(info.fresh, "無法判讀不翻布林：HEAD+age 說 fresh 就是 fresh");
    let a = claudecat::cort::audit_index(&proj).expect("audit_index 應有結果");
    assert_eq!(a.graph_pending, None);
    assert_eq!(a.fresh, info.fresh);
    assert!(
        audit_row(a).contains("fresh?"),
        "日表要看得出「fresh 但圖狀態未知」"
    );
    drop(guard);
}

/// cort 端已有 Java 圖，claudecat 端沒有 grammar：地圖不得讓「空符號」看起來像「沒結構」。
/// 純 Rust 專案不得出現這句（不製造噪音）。
#[test]
fn outline_says_which_key_files_have_no_local_ast() {
    let dir = temp_project();
    fs::create_dir_all(dir.join("src")).unwrap();
    let mut java = String::from("package demo;\n\npublic class OrderService {\n");
    for i in 0..60 {
        java.push_str(&format!("  void run{i}() {{ }}\n"));
    }
    java.push_str("}\n");
    fs::write(dir.join("src/OrderService.java"), &java).unwrap();
    let map = claudecat_lib_scan(&dir);
    let md = claudecat::outline::render_markdown(&map);
    assert!(
        md.contains("`src/OrderService.java`"),
        "java 檔仍要進 key files（改成非 code 會讓 Java 專案整個消失）"
    );
    assert!(
        md.contains("java 檔無本地 AST") && md.contains("navigate --cort"),
        "空符號要明說原因與替代路徑，實際輸出：\n{md}"
    );

    let rust_only = temp_project();
    fs::create_dir_all(rust_only.join("src")).unwrap();
    fs::write(
        rust_only.join("src/main.rs"),
        "fn main() {}\npub fn alpha() {}\n",
    )
    .unwrap();
    let md2 = claudecat::outline::render_markdown(&claudecat_lib_scan(&rust_only));
    assert!(!md2.contains("無本地 AST"), "有 grammar 的專案不得出現這句");
}

/// harness 切面（cortexyoung v3 payload）：router 面對哪個 harness、對誰從沒開過口。
/// 三個必須成立的語意：①沒有 `harness` 欄的歷史列另計，不得攤進任一 harness
/// ②`not_a_search_tool` 是 baseline，不得佔走 top decline
/// ③`harness_declared` 與實測不符要看得見（實測 grok 會宣告成 claude-code）
#[test]
fn cort_audit_usage_splits_by_harness() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, project_id TEXT,
               command TEXT NOT NULL, args_summary TEXT NOT NULL,
               status TEXT NOT NULL, error_code TEXT,
               read_source TEXT, requested_content_mode TEXT, effective_content_mode TEXT,
               receipt_hit INTEGER, index_stale INTEGER,
               bytes_out INTEGER NOT NULL, saved_bytes INTEGER NOT NULL
             );",
        )
        .unwrap();
        let add = |cmd: &str, args: &str| {
            conn.execute(
                "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
                 VALUES (?1, ?2, ?3, 'ok', 0, 0, 0)",
                rusqlite::params![now, cmd, args],
            )
            .unwrap();
        };
        add(
            "hook-suggest",
            r#"{"hook":"hit","harness":"claude-code","v":3}"#,
        );
        add(
            "hook-suggest",
            r#"{"hook":"no_shape","decline":"pattern_not_symbol","harness":"claude-code","v":3}"#,
        );
        // baseline 佔多數也不得成為 top decline
        for _ in 0..3 {
            add(
                "hook-suggest",
                r#"{"hook":"no_shape","decline":"not_a_search_tool","harness":"claude-code","v":3}"#,
            );
        }
        add(
            "hook-suggest",
            r#"{"hook":"no_shape","decline":"unindexed_extension","harness":"codex","v":3}"#,
        );
        add(
            "hook-refresh",
            r#"{"hook":"reindexed","harness":"codex","v":3}"#,
        );
        // 宣告值與實測不符（grok 宣告成 claude-code）
        add(
            "hook-suggest",
            r#"{"hook":"no_shape","decline":"not_a_search_tool","harness":"grok","harness_declared":"claude-code","v":3}"#,
        );
        // v3 之前的歷史列：沒有 harness 欄
        add("hook-suggest", r#"{"hook":"no_shape"}"#);
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    let cc = u.by_harness.get("claude-code").expect("claude-code 應有列");
    assert_eq!((cc.suggests, cc.hits, cc.no_shape), (5, 1, 4));
    assert_eq!(
        cc.top_decline,
        Some(("pattern_not_symbol".to_string(), 1)),
        "baseline 3 筆不得壓過可行動的 1 筆"
    );
    let cx = u.by_harness.get("codex").expect("codex 應有列");
    assert_eq!(
        (cx.suggests, cx.refreshes),
        (1, 1),
        "refresh 不算進 suggest"
    );
    let gk = u.by_harness.get("grok").expect("grok 應有列");
    assert_eq!(gk.declared_mismatch, 1);
    assert_eq!(cc.declared_mismatch, 0, "宣告值相符者不得誤記為不符");
    assert_eq!(u.harness_unknown, 1, "沒有 harness 欄的歷史列必須另計");
    assert_eq!(
        u.by_harness.values().map(|s| s.suggests).sum::<i64>() + u.harness_unknown,
        u.suggest_outcomes.values().sum::<i64>(),
        "各 harness 的 suggests 加總 + unknown 必須對得上 hook-suggest 總數（refresh 走另一欄）"
    );

    // 報告與提示：切面要出現在報告裡，0 命中的 harness 要被指名
    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    };
    let report = claudecat::cort_audit::render(&a);
    assert!(report.contains("harness 切面"), "報告應有 harness 表");
    assert!(
        report.contains("`harness_declared` 與實際不符"),
        "宣告不符要在報告裡說出來"
    );
    drop(guard);
}

/// `unspecified` 是「旗標與 transcript 都認不出來源」的 fallback——實測全是開發/安裝時
/// 手打的探針（2026-09-09 查證），它的命中會墊高總命中率。分母不動（跨日可比），
/// 但污染必須在報告裡講出來，且要給出扣掉之後的那組數字。
#[test]
fn cort_audit_names_unspecified_contamination_in_hit_rate() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-cache-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, project_id TEXT,
               command TEXT NOT NULL, args_summary TEXT NOT NULL,
               status TEXT NOT NULL, error_code TEXT,
               read_source TEXT, requested_content_mode TEXT, effective_content_mode TEXT,
               receipt_hit INTEGER, index_stale INTEGER,
               bytes_out INTEGER NOT NULL, saved_bytes INTEGER NOT NULL
             );",
        )
        .unwrap();
        let add = |args: &str| {
            conn.execute(
                "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
                 VALUES (?1, 'hook-suggest', ?2, 'ok', 0, 0, 0)",
                rusqlite::params![now, args],
            )
            .unwrap();
        };
        // agent 流量：400 筆 no_shape + 1 命中（命中率 <1% 才會觸發那條提示）
        for _ in 0..400 {
            add(
                r#"{"hook":"no_shape","decline":"not_a_search_tool","harness":"claude-code","v":3}"#,
            );
        }
        add(r#"{"hook":"hit","harness":"claude-code","v":3}"#);
        // 手動探針：2 筆命中，會把總命中率從 1/401 墊高到 3/403
        add(r#"{"hook":"hit","harness":"unspecified","v":3}"#);
        add(r#"{"hook":"hit_stale","harness":"unspecified","v":3}"#);
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    assert_eq!(u.by_harness.get("unspecified").map(|s| s.hits), Some(2));
    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    };
    let report = claudecat::cort_audit::render(&a);
    assert!(
        report.contains("hook-suggest 命中率 3/403"),
        "分母/分子不得偷偷改動（跨日可比），實際輸出：\n{report}"
    );
    assert!(
        report.contains("命中數含 2 筆 `unspecified`") && report.contains("agent 實際命中 1／401"),
        "污染與扣除後的數字都要講出來，實際輸出：\n{report}"
    );
    drop(guard);
}

/// 09:29 分析腿每天失敗在 `cc_claude: cannot exec claude: No such file or directory`：
/// 執行檔本身是絕對路徑，但**它 exec 的下一層仍靠 PATH**，而 cron 的 PATH 沒有 ~/.local/bin。
/// 兩個回歸：①條目要把執行檔所在目錄放進 PATH ②模板改了就必須真的重新部署
/// （舊比對只看「執行檔字串在不在」，會把壞條目判成最新，`--install` 安靜地什麼都不做）。
#[test]
fn doctor_analysis_line_carries_path_and_redeploys_on_template_change() {
    let line = claudecat::doctor::analysis_cron_line(
        "/home/u/.local/bin/musecode",
        "/home/u/claudecat",
        &["/home/u/.nvm/versions/node/v24/bin".to_string()],
    );
    assert!(
        line.contains(r#"PATH="/home/u/.local/bin:/home/u/.nvm/versions/node/v24/bin:$PATH""#),
        "執行檔目錄 + runtime 目錄（harness hook 會用名字 exec node）都要在，實際：{line}"
    );
    // 重複目錄不得堆疊
    let dedup = claudecat::doctor::analysis_cron_line(
        "/home/u/.local/bin/musecode",
        "/home/u/claudecat",
        &["/home/u/.local/bin".to_string()],
    );
    assert_eq!(
        dedup.matches("/home/u/.local/bin:").count(),
        1,
        "同一個目錄只出現一次：{dedup}"
    );
    assert!(
        line.starts_with("29 9 * * * PATH="),
        "PATH 要在執行檔之前，實際：{line}"
    );

    // 舊條目＝同一顆執行檔但沒有 PATH 前綴（正是本機 crontab 的壞形狀）
    let broken = "29 9 * * * /home/u/.local/bin/musecode -p --dangerously-skip-permissions \
                  \"$(cat /home/u/claudecat/cort-audit-analysis-prompt.md)\" >> \
                  /home/u/claudecat/cort-audit-analysis.log 2>&1";
    let (out, changed) = claudecat::doctor::merge_analysis_entry(broken, &line);
    assert!(
        changed,
        "模板修好了就必須重寫，不能因為執行檔沒換而視為最新"
    );
    assert!(out.contains("PATH="), "重寫後要帶上修正");
    assert_eq!(out.lines().count(), 1, "不得重複堆疊條目");

    // 已是正確的那行 → 幂等
    let (again, changed2) = claudecat::doctor::merge_analysis_entry(&out, &line);
    assert!(!changed2, "同一行不得反覆重寫");
    assert_eq!(again, out);
}

/// 每日分析的發現要落在**文件**（CLAUDE.md 只留規則，最多一行指引），
/// 且必須與長期指標表共存：整段取代只留最新一天，表格與使用者內容原樣不動。
#[test]
fn findings_replace_section_and_preserve_table_and_notes() {
    let dir = temp_project();
    let f = dir.join("CORT-AUDIT.md");
    fs::write(
        &f,
        "## 長期指標 (claudecat cort-audit)\n\n| 日期 | 專案 |\n|---|---|\n| 2026-09-08 | `/x` |\n\n## 使用者筆記\n- 不要動我\n",
    )
    .unwrap();

    let (changed, _) = claudecat::cort_audit::findings_update(&f, "- 第一天的發現").unwrap();
    assert!(changed);
    let after = fs::read_to_string(&f).unwrap();
    assert!(after.contains("## 每日分析發現 (claudecat cort-audit)"));
    assert!(after.contains("- 第一天的發現"));
    assert!(
        after.contains("| 2026-09-08 | `/x` |") && after.contains("- 不要動我"),
        "表格與使用者筆記不得被吃掉：\n{after}"
    );

    // 第二天：整段取代，不疊成流水帳（歷史在 git）
    let (changed2, _) = claudecat::cort_audit::findings_update(&f, "- 第二天的發現").unwrap();
    assert!(changed2);
    let after2 = fs::read_to_string(&f).unwrap();
    assert!(after2.contains("- 第二天的發現"));
    assert!(
        !after2.contains("- 第一天的發現"),
        "只留最新一天，實際：\n{after2}"
    );
    assert_eq!(
        after2
            .matches("## 每日分析發現 (claudecat cort-audit)")
            .count(),
        1,
        "區塊不得長出第二份"
    );
    assert!(after2.contains("- 不要動我"), "使用者筆記仍在");

    // 同內容再寫一次 → 無變更（不製造無意義的 git diff）
    let (changed3, _) = claudecat::cort_audit::findings_update(&f, "- 第二天的發現").unwrap();
    assert!(!changed3, "內容相同不得改檔");
}

/// v6/v7 形狀的 `file_state` fixture：`v7_cols=false` 時連 `chunk_count` /
/// `indexed_uncommitted` 兩欄都不存在（v5 之前的 DB），這是「讀不到就回 None」那條
/// 鐵則唯一能被驗證的地方——claudecat 是消費者，靠欄位在不在判斷，不靠版本號。
/// `files` = (file_path, chunk_count, indexed_uncommitted, 要不要在 chunks 裡有列)
fn cort_chunk_count_fixture(
    v7_cols: bool,
    files: &[(&str, i64, i64, bool)],
) -> (std::path::PathBuf, std::path::PathBuf) {
    let proj = temp_project();
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-v7-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let real_str = fs::canonicalize(&proj)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let pid = claudecat::cort::project_id(&real_str);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let file_state_cols = if v7_cols {
        "project_id TEXT NOT NULL, file_path TEXT NOT NULL, file_content_hash TEXT NOT NULL, \
         indexed_uncommitted INTEGER NOT NULL DEFAULT 0, chunk_count INTEGER NOT NULL DEFAULT -1"
    } else {
        "project_id TEXT NOT NULL, file_path TEXT NOT NULL, file_content_hash TEXT NOT NULL"
    };
    let conn = rusqlite::Connection::open(cache.join(format!("{pid}.db"))).unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE projects (
           project_id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
           git_head TEXT, last_indexed_at INTEGER, extractor_version TEXT NOT NULL
         );
         CREATE TABLE chunks (
           chunk_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, file_path TEXT NOT NULL,
           symbol_name TEXT, chunk_type TEXT, start_line INTEGER NOT NULL,
           end_line INTEGER NOT NULL, content TEXT NOT NULL, language TEXT,
           chunk_source TEXT DEFAULT 'ast'
         );
         CREATE TABLE file_state ({file_state_cols});
         CREATE TABLE relationships (
           source_chunk_id TEXT NOT NULL, target_chunk_id TEXT NOT NULL,
           rel_type TEXT NOT NULL, call_site_line INTEGER, confidence_score REAL NOT NULL
         );
         INSERT INTO projects VALUES ('{pid}', 'demo', '{real_str}', NULL, {now_ms}, 'test');"
    ))
    .unwrap();
    for (i, (path, chunk_count, uncommitted, chunked)) in files.iter().enumerate() {
        if v7_cols {
            conn.execute(
                "INSERT INTO file_state VALUES (?1, ?2, 'h', ?3, ?4)",
                rusqlite::params![pid, path, uncommitted, chunk_count],
            )
            .unwrap();
        } else {
            conn.execute(
                "INSERT INTO file_state VALUES (?1, ?2, 'h')",
                rusqlite::params![pid, path],
            )
            .unwrap();
        }
        if *chunked {
            conn.execute(
                "INSERT INTO chunks (chunk_id, project_id, file_path, symbol_name, chunk_type, \
                 start_line, end_line, content, language, chunk_source) \
                 VALUES (?1, ?2, ?3, 'sym', 'function', 1, 2, 'body', 'Rust', 'ast')",
                rusqlite::params![format!("c{i}"), pid, path],
            )
            .unwrap();
        }
    }
    drop(conn);
    (proj, cache)
}

/// 在 fixture cache 下跑一次 audit_index——env 改動只活在這個函式裡，
/// 不讓 `CORT_CACHE_DIR` 漏到其他並行測試。
fn audit_index_in(
    proj: &std::path::Path,
    cache: &std::path::Path,
) -> claudecat::cort::CortAuditIndex {
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());
    let a = claudecat::cort::audit_index(proj).expect("audit_index 應有結果");
    drop(guard);
    a
}

fn render_index_report(a: claudecat::cort::CortAuditIndex) -> String {
    claudecat::cort_audit::render(&claudecat::cort::CortAudit {
        root: a.path.clone(),
        host: "test-host".to_string(),
        window_days: 30,
        index: Some(a),
        db_exists: true,
        usage: None,
        usage_7d: None,
    })
}

/// cortexyoung#2 的正確沉默與 #5 的真缺口長得一樣（都是「file_state 有、chunks 沒有」），
/// schema v7 的 `chunk_count` 是唯一能把兩者分開的事實來源：0＝掃過且沒宣告。
/// 這之前 claudecat 只能印「兩種成因都要查」的猜測。
#[test]
fn cort_audit_chunk_count_splits_real_gap_from_correct_silence() {
    let (proj, cache) = cort_chunk_count_fixture(
        true,
        &[
            ("src/silent_a.rs", 0, 0, false),
            ("src/silent_b.rs", 0, 0, false),
            ("src/lost.rs", 5, 0, false),
            ("src/ok.rs", 3, 0, true),
        ],
    );
    let a = audit_index_in(&proj, &cache);
    assert_eq!(a.not_chunked_total, Some(3));
    assert_eq!(
        a.not_chunked_scanned_empty,
        Some(2),
        "chunk_count=0 的正確沉默"
    );
    assert_eq!(a.not_chunked_unknown, Some(0), "v7 fixture 沒有 -1 的列");
    assert_eq!(a.indexed_uncommitted_files, Some(0));
    assert_eq!(
        a.real_gap(),
        Some(1),
        "只有 chunk_count>0 卻沒 chunk 的那檔才是真缺口"
    );

    let report = render_index_report(a);
    assert!(report.contains("cortexyoung#5"), "真缺口要指名上游 issue");
    assert!(report.contains("src/lost.rs"), "真缺口要列出檔名");
    assert!(report.contains("全量"), "修復只能靠全量 cort index");
    assert!(report.contains("cort index"));
}

/// `chunk_count = -1` 是「v7 之前寫入、之後從未重寫」，不是掃描結果——
/// 把它算進正確沉默等於拿舊資料當證據。
#[test]
fn cort_audit_pre_v7_chunk_count_is_unknown_not_a_scan_result() {
    let (proj, cache) = cort_chunk_count_fixture(
        true,
        &[("src/old.rs", -1, 0, false), ("src/silent.rs", 0, 0, false)],
    );
    let a = audit_index_in(&proj, &cache);
    assert_eq!(a.not_chunked_total, Some(2));
    assert_eq!(a.not_chunked_scanned_empty, Some(1));
    assert_eq!(
        a.not_chunked_unknown,
        Some(1),
        "-1 要進 unknown，不進正確沉默"
    );

    let report = render_index_report(a);
    assert!(
        report.contains("不是掃描結果"),
        "報告要明說 -1 不能當成掃過的結論"
    );
    assert!(report.contains("全量"), "要定論只能重新全量索引");
}

/// v5 形狀（`file_state` 連這兩欄都沒有）：三個新欄位皆 None、`real_gap()` None，
/// 報告退回保守措辭——絕不能因為查不到就印「無真缺口」。
#[test]
fn cort_audit_v5_file_state_has_no_chunk_count_columns() {
    let (proj, cache) = cort_chunk_count_fixture(
        false,
        &[("src/mystery.rs", 0, 0, false), ("src/ok.rs", 0, 0, true)],
    );
    let a = audit_index_in(&proj, &cache);
    assert_eq!(a.not_chunked_total, Some(1), "舊欄位照舊可判讀");
    assert_eq!(a.not_chunked_scanned_empty, None);
    assert_eq!(a.not_chunked_unknown, None);
    assert_eq!(a.indexed_uncommitted_files, None);
    assert_eq!(a.real_gap(), None, "任一欄 None 就不准猜");

    let report = render_index_report(a);
    assert!(
        report.contains("無法用欄位判讀"),
        "舊 schema 要說自己判讀不了"
    );
    assert!(!report.contains("無真缺口"), "查不到不等於沒有缺口");
}

/// schema v6 的 `indexed_uncommitted`：索引建立在未提交內容上，git 還原後增量看不到
/// 差異、永不重看（cortexyoung#5 的成因本身）——這件事必須在報告裡警示。
#[test]
fn cort_audit_flags_files_indexed_from_uncommitted_content() {
    let (proj, cache) = cort_chunk_count_fixture(true, &[("src/wip.rs", 3, 1, true)]);
    let a = audit_index_in(&proj, &cache);
    assert_eq!(a.not_chunked_total, Some(0), "這檔有 chunk，不是覆蓋缺口");
    assert_eq!(a.indexed_uncommitted_files, Some(1));

    let report = render_index_report(a);
    assert!(report.contains("未提交"), "要警示索引自未提交內容");
    assert!(report.contains("cortexyoung#5"));
}

/// `shape` 欄（cortexyoung 09f55136）把 no_shape 從「router 沒開口」變成可指名的
/// 工具 × top-level key 形狀。口徑必須與 top declines 一致：baseline 不混進排行，
/// 否則 2000+ 筆「本來就不是搜尋」會把唯一的行動靶心擠掉。
#[test]
fn cort_audit_usage_tracks_no_shape_shapes() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-shape-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, project_id TEXT,
               command TEXT NOT NULL, args_summary TEXT NOT NULL,
               status TEXT NOT NULL, error_code TEXT, index_stale INTEGER,
               bytes_out INTEGER NOT NULL, saved_bytes INTEGER NOT NULL
             );",
        )
        .unwrap();
        let ins = |summary: &str| {
            conn.execute(
                "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
                 VALUES (?1, 'hook-suggest', ?2, 'ok', 0, 0, 0)",
                rusqlite::params![now, summary],
            )
            .unwrap();
        };
        // baseline：有 shape 也不得進排行
        ins(r#"{"hook":"no_shape","decline":"not_a_search_tool","shape":"Bash|cwd+tool_name"}"#);
        ins(r#"{"hook":"no_shape","decline":"not_a_search_tool","shape":"Bash|cwd+tool_name"}"#);
        // actionable 兩種形狀
        ins(
            r#"{"hook":"no_shape","decline":"pattern_not_symbol","shape":"Grep|pattern+tool_name"}"#,
        );
        ins(
            r#"{"hook":"no_shape","decline":"pattern_not_symbol","shape":"Grep|pattern+tool_name"}"#,
        );
        ins(r#"{"hook":"no_shape","decline":"context_flag","shape":"Read|file_path+tool_name"}"#);
        // 09f55136 之前的舊列沒有 shape → 跳過，不得記成 "unparsed" 汙染排行
        ins(r#"{"hook":"no_shape","decline":"context_flag"}"#);
        // 命中列即使帶 shape 也與 no_shape 無關
        ins(r#"{"hook":"hit","shape":"Grep|pattern+tool_name"}"#);
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    assert_eq!(u.no_shape_shapes.get("Grep|pattern+tool_name"), Some(&2));
    assert_eq!(u.no_shape_shapes.get("Read|file_path+tool_name"), Some(&1));
    assert_eq!(
        u.no_shape_shapes.get("Bash|cwd+tool_name"),
        None,
        "baseline 不得進 shape 排行"
    );
    assert_eq!(
        u.no_shape_shapes.len(),
        2,
        "無 shape 的舊列不得造出第三個鍵"
    );
    assert_eq!(u.no_shape_shapes.get("unparsed"), None);

    let report = claudecat::cort_audit::render(&claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    });
    assert!(report.contains("top no_shape shapes"), "報告要有 shape 段");
    assert!(
        report.contains("`Grep|pattern+tool_name`"),
        "shape 字串長，要用反引號包住"
    );
    drop(guard);
}

/// `shape` 是 cortexyoung 09f55136（2026-09-09 11:05）才開始寫的欄位，30 天窗裡
/// 絕大多數 actionable no_shape 列根本沒有它（實測 20/6904）。拿 actionable 當分母
/// 會把 11 筆算成 0.2% → `{:.0}` 印成 **0%**，等於把唯一的行動靶心標成無關緊要。
/// 分母必須與分子同一母體（實際帶 shape 的列），兩個母體的關係另行印出來。
#[test]
fn cort_audit_shape_percentage_uses_shape_carrying_rows_as_denominator() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-cort-shapepct-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, project_id TEXT,
               command TEXT NOT NULL, args_summary TEXT NOT NULL,
               status TEXT NOT NULL, error_code TEXT, index_stale INTEGER,
               bytes_out INTEGER NOT NULL, saved_bytes INTEGER NOT NULL
             );",
        )
        .unwrap();
        let ins = |summary: &str| {
            conn.execute(
                "INSERT INTO command_log (ts, command, args_summary, status, index_stale, bytes_out, saved_bytes) \
                 VALUES (?1, 'hook-suggest', ?2, 'ok', 0, 0, 0)",
                rusqlite::params![now, summary],
            )
            .unwrap();
        };
        // 700 筆 actionable 但沒有 shape（09f55136 之前的舊列）——比例刻意做到
        // 覆蓋率 <0.5%，好釘住「覆蓋率自己也不准被 {:.0} 四捨五入成 0%」
        for _ in 0..700 {
            ins(r#"{"hook":"no_shape","decline":"pattern_not_symbol"}"#);
        }
        // 只有 3 筆帶 shape——真實比例應以這 3 筆為母體
        ins(
            r#"{"hook":"no_shape","decline":"pattern_not_symbol","shape":"Grep|pattern+tool_name"}"#,
        );
        ins(
            r#"{"hook":"no_shape","decline":"pattern_not_symbol","shape":"Grep|pattern+tool_name"}"#,
        );
        ins(r#"{"hook":"no_shape","decline":"context_flag","shape":"Read|file_path+tool_name"}"#);
        // baseline 帶不帶 shape 都不進母體
        for _ in 0..5 {
            ins(
                r#"{"hook":"no_shape","decline":"not_a_search_tool","shape":"Bash|cwd+tool_name"}"#,
            );
        }
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());
    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    drop(guard);

    assert_eq!(
        u.no_shape_shapes.values().sum::<i64>(),
        3,
        "母體是帶 shape 的列"
    );

    let report = claudecat::cort_audit::render(&claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    });
    assert!(
        report.contains("67%"),
        "2/3 要印成 67%，不是 2/23 的 0%：\n{report}"
    );
    assert!(report.contains("33%"), "1/3 = 33%");
    // 只看 shape 那幾行：既有的 declines 段用「佔 no_shape」當標籤，分母與標籤相符，
    // 是另一回事（見回報的已知限制），不在這個測試的射程內。
    for line in report.lines().filter(|l| l.contains("佔帶 shape")) {
        assert!(!line.contains("0%）"), "shape 排行不得被壓成 0%: {line}");
    }
    // 覆蓋率那行要同時講出兩個母體，讓讀者知道 67% 是在多小的樣本上成立的
    let line = report
        .lines()
        .find(|l| l.contains("母體"))
        .expect("要有覆蓋率行");
    assert!(line.contains('3'), "帶 shape 的列數: {line}");
    assert!(line.contains("703"), "actionable no_shape 總數: {line}");
    // 覆蓋率 3/703 = 0.4%——連這一行都不准被四捨五入成「覆蓋 0%」，
    // 那正是這個測試在修的那個謊言的另一半
    assert!(!line.contains("覆蓋 0%"), "覆蓋率不得四捨五入成 0%: {line}");
    assert!(line.contains("0.4"), "覆蓋率要有小數位: {line}");
}

/// 欄數漂移不該靠人眼抓：2026-09-06 那列少一格 host 已經證明過代價。
/// header / 對齊列 / 資料列三者的欄數必須永遠相等。
#[test]
fn cort_audit_track_table_column_counts_match() {
    fn cells(line: &str) -> usize {
        line.trim()
            .trim_start_matches('|')
            .trim_end_matches('|')
            .split('|')
            .count()
    }
    let (proj, cache) = cort_chunk_count_fixture(true, &[("src/ok.rs", 3, 0, true)]);
    let idx = audit_index_in(&proj, &cache);
    let a = claudecat::cort::CortAudit {
        root: idx.path.clone(),
        host: "test-host".to_string(),
        window_days: 30,
        index: Some(idx),
        db_exists: true,
        usage: None,
        usage_7d: None,
    };
    let target = std::env::temp_dir().join(format!(
        "claudecat-track-cols-{}-{}.md",
        std::process::id(),
        rand_suffix()
    ));
    claudecat::cort_audit::track_update(&target, &[&a]).unwrap();
    let content = fs::read_to_string(&target).unwrap();
    let header = content
        .lines()
        .find(|l| l.starts_with("| 日期"))
        .expect("要有 header");
    let align = content
        .lines()
        .find(|l| l.starts_with("|---"))
        .expect("要有對齊列");
    let row = claudecat::cort_audit::row_md(&a);
    assert_eq!(cells(header), cells(align), "header 與對齊列欄數必須一致");
    assert_eq!(cells(header), cells(&row), "資料列欄數必須與 header 一致");
    // 兩欄並存：未 chunk 總數（歷史連續）與真缺口（新語意），不是換名
    assert!(header.contains("未chunk檔"), "既有欄名原樣保留");
    assert!(header.contains("真缺口"), "真缺口是新增欄");
    let _ = fs::remove_file(&target);
}

/// hook census（cortexyoung 6623113d 口徑）：每列 hook command_log 恰落一桶，
/// buckets 加總 == fires。合成 usage.db 實測 audit_usage 的分割。
#[test]
fn usage_census_partitions_every_fire() {
    use claudecat::cort::audit_usage;

    let cache = std::env::temp_dir().join(format!(
        "claudecat-census-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE command_log (
           ts INTEGER, command TEXT, status TEXT, args_summary TEXT,
           index_stale INTEGER DEFAULT 0, saved_bytes INTEGER DEFAULT 0
         );",
    )
    .unwrap();
    let now: i64 = 17_890_000_000_000;
    let ins = |cmd: &str, status: &str, summary: &str| {
        conn.execute(
            "INSERT INTO command_log (ts, command, status, args_summary) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![now, cmd, status, summary],
        )
        .unwrap();
    };
    // hook-suggest 八桶
    ins("hook-suggest", "ok", r#"{"hook":"hit"}"#);
    ins(
        "hook-suggest",
        "ok",
        r#"{"hook":"no_shape","decline":"pattern_not_symbol"}"#,
    );
    ins("hook-suggest", "ok", r#"{"hook":"no_shape"}"#); // 缺 decline 欄
    ins("hook-suggest", "error", r#"{"hook":"hit"}"#);
    ins("hook-suggest", "ok", "not json at all");
    ins("hook-suggest", "ok", r#"{"foo":1}"#); // JSON 但沒有 hook 欄
    ins(
        "hook-suggest",
        "ok",
        r#"{"hook":"no_shape","decline":"not_a_search_tool"}"#,
    );
    ins("hook-suggest", "ok", r#"{"hook":"no_evidence"}"#);
    // hook-refresh 四列：refresh 詞彙內、no_shape 不展開（→unknown/no_shape）、詞彙外
    ins("hook-refresh", "ok", r#"{"hook":"refreshed"}"#);
    ins("hook-refresh", "ok", r#"{"hook":"no_shape"}"#);
    ins("hook-refresh", "ok", r#"{"hook":"rebuild_required"}"#);
    ins("hook-refresh", "error", "garbage");
    // 非 hook 命令不進 census
    ins("context", "ok", "whatever");
    drop(conn);

    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = audit_usage(30).expect("合成 usage.db 應可讀");
    drop(guard);

    let sug = u.census.get("hook-suggest").expect("suggest 應有 census");
    assert_eq!(sug.get("_total"), Some(&8));
    let bucket = |k: &str| sug.get(k).copied().unwrap_or(0);
    assert_eq!(bucket("hit"), 1);
    assert_eq!(bucket("no_shape/pattern_not_symbol"), 1);
    assert_eq!(bucket("no_shape/decline_absent"), 1, "缺 decline 欄要現形");
    assert_eq!(bucket("no_shape/not_a_search_tool"), 1);
    assert_eq!(bucket("no_evidence"), 1);
    assert_eq!(bucket("status_error"), 1);
    assert_eq!(bucket("unparseable_summary"), 1);
    assert_eq!(bucket("legacy_unsplit"), 1);
    let sug_sum: i64 = sug
        .iter()
        .filter(|(k, _)| *k != "_total")
        .map(|(_, v)| v)
        .sum();
    assert_eq!(sug_sum, 8, "suggest buckets 加總必須等於 fires");

    let refr = u.census.get("hook-refresh").expect("refresh 應有 census");
    assert_eq!(refr.get("_total"), Some(&4));
    assert_eq!(refr.get("refreshed"), Some(&1));
    assert_eq!(refr.get("rebuild_required"), Some(&1));
    assert_eq!(
        refr.get("unknown/no_shape"),
        Some(&1),
        "refresh 列的 no_shape 不展開 decline，落 unknown/"
    );
    assert_eq!(refr.get("status_error"), Some(&1));
    let ref_sum: i64 = refr
        .iter()
        .filter(|(k, _)| *k != "_total")
        .map(|(_, v)| v)
        .sum();
    assert_eq!(ref_sum, 4, "refresh buckets 加總必須等於 fires");

    assert!(!u.census.contains_key("context"), "非 hook 命令不進 census");
    // 既有切面不受影響：not json 與沒 hook 欄的列在舊 suggest_outcomes 口徑都叫
    // "unparsed"（census 把兩者分開成 unparseable_summary / legacy_unsplit）
    assert_eq!(u.suggest_outcomes.get("unparsed"), Some(&2));
}

/// query-time self-heal 採樣（cortexyoung 5f6d5267）：impact/context 回答前自癒 index，
/// heal 欄位只在「有話要說」的列上（綠路 payload 完全不加 key，heal.rs `attach_to`）。
/// 四個必須成立的語意：①healed 按 mode 分桶、heal_ms 合計/max 正確
/// ②deferred 按理由字串分桶 ③無 heal key 的歷史列進 legacy、不污染新桶
/// ④`index --heal-background` 計背景重建、一般 index 列不計；NULL／非法 JSON 不 panic 不中斷。
#[test]
fn cort_audit_usage_samples_heal_fields() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-heal-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               ts INTEGER, command TEXT, status TEXT, args_summary,
               index_stale INTEGER DEFAULT 0, saved_bytes INTEGER DEFAULT 0
             );",
        )
        .unwrap();
        let ins = |cmd: &str, summary: Option<&str>| {
            conn.execute(
                "INSERT INTO command_log (ts, command, status, args_summary) VALUES (?1, ?2, 'ok', ?3)",
                rusqlite::params![now, cmd, summary],
            )
            .unwrap();
        };
        // healed：heal_mode 與 heal_ms 都要入帳
        ins(
            "impact",
            Some(
                r#"{"symbol":"logInfo","v":1,"self_healed":true,"heal_mode":"full","heal_ms":1500}"#,
            ),
        );
        ins(
            "context",
            Some(r#"{"v":1,"self_healed":true,"heal_mode":"incremental","heal_ms":250}"#),
        );
        // deferred：self_healed=false + 理由字串
        ins(
            "context",
            Some(r#"{"symbol":"x","self_healed":false,"heal_deferred":"background_spawned"}"#),
        );
        // 5f6d5267 之前的歷史列：沒有任何 heal key → legacy，不混進新桶
        ins("impact", Some(r#"{"symbol":"logInfo","v":1}"#));
        // args_summary 為 NULL：不 panic、不中斷，進自己的桶
        ins("impact", None);
        // 背景重建事件 vs 一般 index 列
        ins("index", Some(r#"{"v":1,"heal":"background"}"#));
        ins("index", Some(r#"{"v":1}"#));
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    drop(guard);

    // 分母 = impact+context 列數（4 筆有 summary + 1 筆 NULL）
    assert_eq!(u.heal_scanned, 5);
    assert_eq!(u.heal_self_healed, 2);
    assert_eq!(u.heal_modes.get("full"), Some(&1));
    assert_eq!(u.heal_modes.get("incremental"), Some(&1));
    assert_eq!(u.heal_ms_total, 1750);
    assert_eq!(u.heal_ms_max, 1500);
    assert_eq!(
        u.heal_deferred.get("background_spawned"),
        Some(&1),
        "deferred 按理由字串分桶"
    );
    assert_eq!(u.heal_legacy, 1, "無 heal key 的歷史列只進 legacy");
    assert_eq!(
        u.heal_background, 1,
        "只有帶 heal:background 的 index 列計入"
    );
    assert_eq!(u.heal_unparseable, 1, "NULL summary 可見、不靜默丟棄");
    // 閉合：scanned 恰分到 healed + deferred + legacy + unparseable 四個去處
    assert_eq!(
        u.heal_scanned,
        u.heal_self_healed
            + u.heal_legacy
            + u.heal_unparseable
            + u.heal_deferred.values().sum::<i64>(),
        "分割閉合，沒有列在掃描路上消失"
    );

    // 報告：各桶都要出現
    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    };
    let report = claudecat::cort_audit::render(&a);
    assert!(report.contains("self-heal 採樣"), "報告應有 heal 採樣段");
    assert!(report.contains("scanned=5"), "分母要出現");
    assert!(report.contains("self_healed=2"), "healed 計數要出現");
    assert!(
        report.contains("full=1") && report.contains("incremental=1"),
        "mode 分桶要出現"
    );
    assert!(
        report.contains("background_spawned=1"),
        "deferred 理由要出現"
    );
    assert!(
        report.contains("合計=1750") && report.contains("max=1500"),
        "heal_ms 合計/max 要出現"
    );
    assert!(report.contains("legacy=1"), "legacy 要出現");
    assert!(report.contains("背景重建"), "background 次數要出現");
}

/// 零樣本（窗內 impact/context 全是 legacy 舊列）也要印 scanned：
/// 「0 次自癒」與「還沒資料（legacy=N）」必須分得開，不能混成同一種沉默。
#[test]
fn cort_audit_heal_zero_sample_still_shows_scanned() {
    let cache = std::env::temp_dir().join(format!(
        "claudecat-heal-zero-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    fs::create_dir_all(&cache).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    {
        let conn = rusqlite::Connection::open(cache.join("usage.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE command_log (
               ts INTEGER, command TEXT, status TEXT, args_summary,
               index_stale INTEGER DEFAULT 0, saved_bytes INTEGER DEFAULT 0
             );",
        )
        .unwrap();
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO command_log (ts, command, status, args_summary) \
                 VALUES (?1, 'impact', 'ok', '{\"symbol\":\"x\",\"v\":1}')",
                [&now],
            )
            .unwrap();
        }
    }
    let _env_serial = cort_env_lock();
    let guard = EnvVarGuard(
        "CORT_CACHE_DIR".to_string(),
        std::env::var("CORT_CACHE_DIR").ok(),
    );
    std::env::set_var("CORT_CACHE_DIR", cache.to_str().unwrap());

    let u = claudecat::cort::audit_usage(30).expect("usage 應有結果");
    drop(guard);

    assert_eq!(u.heal_scanned, 3);
    assert_eq!(u.heal_self_healed, 0, "舊列不是自癒，是還沒資料");
    assert_eq!(u.heal_legacy, 3);

    let a = claudecat::cort::CortAudit {
        root: "/tmp/fake-root".to_string(),
        host: "test-host".to_string(),
        window_days: 30,
        index: None,
        db_exists: false,
        usage: Some(u),
        usage_7d: None,
    };
    let report = claudecat::cort_audit::render(&a);
    assert!(
        report.contains("scanned=3")
            && report.contains("self_healed=0")
            && report.contains("legacy=3"),
        "零樣本仍要顯示 scanned，讓 0 次自癒與 legacy=N 分得開：\n{report}"
    );
}

/// repair token（cortexyoung f4ad4c7d）從 `cort status` JSON 的推導，
/// 鏡射 impact.rs：!stale→none；forbid 會拒絕（rebuild_required 非空或
/// candidates_narrowed=false）→ rebuild_required；其餘 → refreshable。
#[test]
fn repair_token_mirrors_upstream_forbid_refuses() {
    let f = claudecat::cort::repair_from_status_json;
    let none = serde_json::json!({"indexed": true, "index_is_stale": false});
    assert_eq!(f(&none).as_deref(), Some("none"));
    let refreshable = serde_json::json!({
        "indexed": true, "index_is_stale": true,
        "rebuild_required": [], "candidates_narrowed": true
    });
    assert_eq!(f(&refreshable).as_deref(), Some("refreshable"));
    let rebuild = serde_json::json!({
        "indexed": true, "index_is_stale": true,
        "rebuild_required": ["extractor_changed"], "candidates_narrowed": true
    });
    assert_eq!(f(&rebuild).as_deref(), Some("rebuild_required"));
    let narrowed = serde_json::json!({
        "indexed": true, "index_is_stale": true,
        "rebuild_required": [], "candidates_narrowed": false
    });
    assert_eq!(
        f(&narrowed).as_deref(),
        Some("rebuild_required"),
        "narrowing 失守時 hook 一樣拒絕，只有全量會修"
    );
    let unindexed = serde_json::json!({
        "indexed": false, "index_is_stale": true,
        "rebuild_required": [], "candidates_narrowed": true
    });
    assert_eq!(f(&unindexed), None, "沒有索引就沒有 repair 可講");
    let unreadable = serde_json::json!({"indexed": true});
    assert_eq!(f(&unreadable), None, "關鍵欄位缺席＝無法判讀，不是 none");
}
