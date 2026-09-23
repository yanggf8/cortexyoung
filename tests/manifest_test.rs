mod common;
use claudecat::manifest;

#[test]
fn package_json_full_parse() {
    let t = common::Tmp(common::temp_dir());
    common::write(
        &t.0,
        "package.json",
        r#"{"name":"api","main":"src/server.js","bin":{"cc":"src/cli.js"},"scripts":{"start":"node src/server.js","build":"tsc"},"dependencies":{"express":"^4","zod":"^3"},"devDependencies":{"typescript":"^5"}}"#,
    );
    let (meta, deps) = manifest::detect_project_meta(&t.0);
    assert_eq!(meta.name, "api");
    assert_eq!(meta.language, "TypeScript/JavaScript");
    assert!(meta
        .entry_points
        .iter()
        .any(|e| e.contains("src/server.js")));
    assert!(meta.entry_points.iter().any(|e| e.contains("src/cli.js")));
    assert!(meta.framework.contains("Express.js"));
    assert_eq!(
        meta.run_command.as_deref(),
        Some("npm start (node src/server.js)")
    );
    let npm = deps.iter().find(|g| g.ecosystem == "npm").unwrap();
    assert!(npm.deps.contains(&"express".to_string()));
    assert!(npm.deps.contains(&"typescript".to_string()));
}

#[test]
fn pyproject_and_requirements() {
    let t = common::Tmp(common::temp_dir());
    common::write(&t.0, "pyproject.toml", "[project]\nname=\"demo\"\ndependencies=[\"fastapi>=0.1\",\"uvicorn\"]\n[project.scripts]\ndemo=\"demo.main:main\"\n");
    common::write(
        &t.0,
        "requirements.txt",
        "requests==2.0\n# comment\nflask\n",
    );
    let (meta, deps) = manifest::detect_project_meta(&t.0);
    assert_eq!(meta.language, "Python");
    assert!(meta.framework.contains("FastAPI"));
    assert!(meta.entry_points.iter().any(|e| e.starts_with("demo")));
    let pip = deps.iter().find(|g| g.ecosystem == "PyPI").unwrap();
    assert!(pip.deps.contains(&"flask".to_string()));
    assert!(pip.deps.contains(&"requests".to_string()));
}

#[test]
fn go_mod_detection() {
    let t = common::Tmp(common::temp_dir());
    common::write(&t.0, "go.mod", "module github.com/me/proj\ngo 1.22\n");
    common::write(&t.0, "main.go", "package main\n");
    let (meta, _) = manifest::detect_project_meta(&t.0);
    assert_eq!(meta.language, "Go");
    assert_eq!(meta.name, "github.com/me/proj");
    assert!(meta.entry_points.iter().any(|e| e == "main.go"));
}
