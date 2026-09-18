mod common;
use claudecat::walk;

#[test]
fn analyze_project_counts_files_loc_and_langs() {
    let t = common::Tmp(common::temp_dir());
    common::write(&t.0, "package.json", "{\"name\":\"x\"}");
    common::write(&t.0, "src/a.js", "const a = 1;\n\nconst b = 2;\n");
    common::write(&t.0, "src/lib/b.py", "# hi\ndef f():\n    pass\n\n\n");
    common::write(&t.0, "src/lib/c.ts", "export function f() {}\n");
    common::write(&t.0, "node_modules/pkg/index.js", "evil();\n");
    common::write(&t.0, "legacy/old.ts", "// archived\n");

    let map = walk::analyze_project(&t.0, 10, None);

    // legacy/node_modules excluded
    assert!(!map.key_files.iter().any(|f| f.path.contains("legacy")));
    assert!(!map
        .key_files
        .iter()
        .any(|f| f.path.contains("node_modules")));
    // package.json 是 config：計入 total_files，但不計 code LOC、不進 key_files
    assert_eq!(map.total_files, 4, "package.json + 3 code files");
    assert!(map.languages.contains_key("javascript"));
    assert!(map.languages.contains_key("python"));
    assert!(map.languages.contains_key("typescript"));
    // LOC: a.js=2, b.py=2(non-blank: def f / pass), c.ts=1
    let js = map
        .key_files
        .iter()
        .find(|f| f.path.ends_with("a.js"))
        .unwrap();
    assert_eq!(js.loc, 2);
    let py = map
        .key_files
        .iter()
        .find(|f| f.path.ends_with("b.py"))
        .unwrap();
    assert_eq!(py.loc, 3); // "# hi", "def f()", "    pass"
    assert_eq!(map.total_loc, 6); // a.js 2 + b.py 3 + c.ts 1 (config 不列入)

    // dir stats for src（只含 code 直接子檔）
    assert!(map.dir_stats.contains_key("src"));
    // 雙重計數修復：src + src/lib 的 LOC 總和必須等於 total_loc
    let src_loc = map.dir_stats.get("src").map(|d| d.loc).unwrap_or(0);
    let lib_loc = map.dir_stats.get("src/lib").map(|d| d.loc).unwrap_or(0);
    assert_eq!(
        src_loc + lib_loc,
        map.total_loc,
        "dir LOC sum must equal total_loc"
    );
}

#[test]
fn top_n_limits_key_files() {
    let t = common::Tmp(common::temp_dir());
    for i in 0..6 {
        common::write(
            &t.0,
            &format!("src/f{i}.rs"),
            &format!("pub fn f{i}() {{}}\n"),
        );
    }
    let map = walk::analyze_project(&t.0, 3, None);
    assert_eq!(map.key_files.len(), 3);
    assert!(map.key_files.iter().all(|f| f.loc == 1));
}

#[test]
fn markdown_headings_are_indexed_without_changing_code_stats() {
    let dir = common::Tmp(common::temp_dir());
    std::fs::write(
        dir.0.join("README.md"),
        "---\ntitle: demo\n---\n# Root\ntext\n```\n# not a heading\n```\n## Child\nmore\n# Next\n",
    )
    .unwrap();
    std::fs::write(dir.0.join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir_all(dir.0.join("legacy")).unwrap();
    std::fs::write(dir.0.join("legacy/old.md"), "# Ignored\n").unwrap();
    let map = claudecat::walk::analyze_project(&dir.0, 10, None);
    let headings = claudecat::walk::collect_markdown_files(&dir.0)
        .iter()
        .flat_map(|path| claudecat::markdown::index_file(&dir.0, path))
        .collect::<Vec<_>>();
    assert_eq!(map.total_files, 1);
    assert_eq!(
        headings
            .iter()
            .map(|h| h.heading_path.join(" > "))
            .collect::<Vec<_>>(),
        vec!["Root", "Root > Child", "Next"]
    );
    assert_eq!(headings[0].end_line, 10);
    assert_eq!(headings[1].end_line, 10);
    assert!(!headings
        .iter()
        .any(|heading| heading.path.contains("legacy")));
}

#[test]
fn tree_lines_compacts_structure() {
    let t = common::Tmp(common::temp_dir());
    common::write(&t.0, "src/a.rs", "fn a(){}\n");
    common::write(&t.0, "src/sub/b.rs", "fn b(){}\n");
    common::write(&t.0, "tests/t.rs", "fn t(){}\n");
    let map = walk::analyze_project(&t.0, 10, None);
    let lines = walk::tree_lines(&map.dir_stats, 2, 40, map.total_loc);
    let joined = lines.join("\n");
    assert!(joined.contains("src/"));
    assert!(joined.contains("tests/"));
    assert!(joined.contains("LOC"));
}
