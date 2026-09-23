//! Manifest detection: package.json / Cargo.toml / pyproject.toml / go.mod / requirements.txt
use crate::model::{DepGroup, ProjectMeta};
use std::path::Path;

/// 主要語言優先序（dual manifest 時決定 primary，避免「Rust 專案標成 Node」）
const PRIMARY_RANK: &[(&str, &str)] = &[
    ("Cargo.toml", "cargo"),
    ("go.mod", "go"),
    ("pyproject.toml", "pip"),
    ("package.json", "npm"),
    ("Gemfile", "bundler"),
    ("composer.json", "composer"),
];

pub fn detect_project_meta(root: &Path) -> (ProjectMeta, Vec<DepGroup>) {
    let dir_name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());

    let mut deps: Vec<DepGroup> = Vec::new();
    let mut meta = ProjectMeta::default();

    // 1) 先收集所有 manifest 的資訊與 deps
    let mut candidates: Vec<(usize, ProjectMeta, Option<DepGroup>)> = Vec::new();
    if root.join("package.json").is_file() {
        if let Some((m, d)) = parse_package_json(&root.join("package.json")) {
            candidates.push((rank_of("package.json"), m, Some(d)));
        }
    }
    if root.join("Cargo.toml").is_file() {
        if let Some((m, d)) = parse_cargo_toml(&root.join("Cargo.toml")) {
            candidates.push((rank_of("Cargo.toml"), m, Some(d)));
        }
    }
    if root.join("pyproject.toml").is_file() {
        if let Some((m, d)) = parse_pyproject(&root.join("pyproject.toml")) {
            candidates.push((rank_of("pyproject.toml"), m, Some(d)));
        }
    }
    if root.join("requirements.txt").is_file() {
        if let Some(d) = parse_requirements(&root.join("requirements.txt")) {
            deps.push(d);
        }
    }
    if root.join("go.mod").is_file() {
        if let Some((m, d)) = parse_go_mod(&root.join("go.mod")) {
            candidates.push((rank_of("go.mod"), m, Some(d)));
        }
    }
    if root.join("Gemfile").is_file() {
        if let Some((m, d)) = parse_gemfile(&root.join("Gemfile")) {
            candidates.push((rank_of("Gemfile"), m, Some(d)));
        }
    }
    if root.join("composer.json").is_file() {
        if let Some((m, d)) = parse_composer(&root.join("composer.json")) {
            candidates.push((rank_of("composer.json"), m, Some(d)));
        }
    }

    // 2) primary = rank 最小者；其餘 manifest 的資訊只補 deps，不改 primary 語意
    candidates.sort_by_key(|(rank, _, _)| *rank);
    if let Some((_, primary, primary_deps)) = candidates.first() {
        meta = primary.clone();
        if let Some(d) = primary_deps {
            deps.push(d.clone());
        }
    }
    for (_, _other, other_deps) in candidates.iter().skip(1) {
        if let Some(d) = other_deps {
            deps.push(d.clone());
        }
    }

    // 3) package manager 依 lockfile 精確化（npm/yarn/pnpm/uv/pip）
    refine_package_manager(root, &mut meta);

    // 4) 合併重複 ecosystem（例如 go.mod + 手動列）
    let mut merged: Vec<DepGroup> = Vec::new();
    for g in deps {
        if let Some(existing) = merged.iter_mut().find(|e| e.ecosystem == g.ecosystem) {
            existing.deps.extend(g.deps);
            existing.deps.sort();
            existing.deps.dedup();
        } else {
            merged.push(g);
        }
    }
    let deps = merged;

    if meta.name.is_empty() {
        meta.name = dir_name;
    }
    if meta.entry_points.is_empty() {
        for guess in [
            "src/main.rs",
            "main.py",
            "src/main.py",
            "__main__.py",
            "main.go",
            "src/main.go",
            "index.js",
            "index.ts",
            "src/index.js",
            "src/index.ts",
            "app.js",
            "app.py",
            "server.js",
            "src/server.js",
        ] {
            if root.join(guess).is_file() {
                meta.entry_points.push(guess.to_string());
            }
        }
    }
    infer_framework(&mut meta, &deps);
    (meta, deps)
}

fn rank_of(manifest: &str) -> usize {
    PRIMARY_RANK
        .iter()
        .position(|(m, _)| *m == manifest)
        .unwrap_or(usize::MAX)
}

fn refine_package_manager(root: &Path, meta: &mut ProjectMeta) {
    if root.join("pnpm-lock.yaml").is_file() {
        meta.package_manager = "pnpm".into();
    } else if root.join("yarn.lock").is_file() {
        meta.package_manager = "yarn".into();
    } else if root.join("package-lock.json").is_file() {
        meta.package_manager = "npm".into();
    } else if root.join("uv.lock").is_file() {
        meta.package_manager = "uv".into();
    }
}

fn parse_package_json(path: &Path) -> Option<(ProjectMeta, DepGroup)> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut meta = ProjectMeta {
        project_type: "Node.js application/library".into(),
        package_manager: "npm".into(),
        name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").into(),
        language: "TypeScript/JavaScript".into(),
        ..ProjectMeta::default()
    };

    if let Some(bins) = v.get("bin") {
        let mut entries = vec![];
        if let Some(s) = bins.as_str() {
            entries.push(s.to_string());
        } else if let Some(map) = bins.as_object() {
            for (k, p) in map {
                if let Some(p) = p.as_str() {
                    entries.push(format!("{k} -> {p}"));
                }
            }
        }
        meta.entry_points.extend(entries);
    }
    if let Some(main) = v.get("main").and_then(|x| x.as_str()) {
        meta.entry_points.push(main.to_string());
    }
    if let Some(scripts) = v.get("scripts").and_then(|x| x.as_object()) {
        for (k, val) in scripts {
            if let Some(val) = val.as_str() {
                let key = k.clone();
                let value = val.to_string();
                match key.as_str() {
                    "start" => {
                        meta.run_command = Some(format!("npm start ({value})"));
                    }
                    "dev" => {
                        meta.run_command = meta
                            .run_command
                            .clone()
                            .or(Some(format!("npm run dev ({value})")));
                    }
                    "build" => {
                        meta.build_command = Some(format!("npm run build ({value})"));
                    }
                    _ => {}
                }
                meta.scripts.insert(key, value);
            }
        }
    }

    let mut deps: Vec<String> = Vec::new();
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = v.get(key).and_then(|x| x.as_object()) {
            for name in map.keys() {
                deps.push(name.clone());
            }
        }
    }
    deps.sort();
    deps.dedup();
    let group = DepGroup {
        ecosystem: "npm".into(),
        deps,
    };
    Some((meta, group))
}

fn parse_cargo_toml(path: &Path) -> Option<(ProjectMeta, DepGroup)> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let mut meta = ProjectMeta {
        project_type: "Rust application/library".into(),
        package_manager: "cargo".into(),
        language: "Rust".into(),
        ..ProjectMeta::default()
    };
    if let Some(pkg) = v.get("package") {
        meta.name = pkg
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into();
        if let Some(bin_path) = pkg.get("default-run").and_then(|x| x.as_str()) {
            meta.entry_points.push(bin_path.to_string());
        }
    }
    if let Some(bins) = v.get("bin").and_then(|x| x.as_array()) {
        for b in bins {
            let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let path = b.get("path").and_then(|x| x.as_str()).unwrap_or("");
            meta.entry_points.push(format!("{name} ({path})"));
        }
    }
    // workspace members：入口併入
    if let Some(ws) = v
        .get("workspace")
        .and_then(|x| x.get("members"))
        .and_then(|x| x.as_array())
    {
        for mem in ws {
            if let Some(m) = mem.as_str() {
                let mp = path.parent().map(|p| p.join(m)).unwrap_or_default();
                if mp.join("src/main.rs").is_file() {
                    meta.entry_points.push(format!("{m}/src/main.rs"));
                }
            }
        }
    }
    if path
        .parent()
        .map(|p| p.join("src/main.rs").is_file())
        .unwrap_or(false)
    {
        meta.entry_points.push("src/main.rs".into());
    }
    meta.run_command = Some("cargo run".into());
    meta.build_command = Some("cargo build".into());

    let mut deps: Vec<String> = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(map) = v.get(section).and_then(|x| x.as_table()) {
            for name in map.keys() {
                deps.push(name.clone());
            }
        }
    }
    deps.sort();
    deps.dedup();
    let group = DepGroup {
        ecosystem: "crates.io".into(),
        deps,
    };
    Some((meta, group))
}

fn parse_pyproject(path: &Path) -> Option<(ProjectMeta, DepGroup)> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    let mut meta = ProjectMeta {
        project_type: "Python application/library".into(),
        package_manager: "uv/pip".into(),
        language: "Python".into(),
        ..ProjectMeta::default()
    };
    if let Some(proj) = v.get("project") {
        meta.name = proj
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into();
        if let Some(scripts) = proj.get("scripts").and_then(|x| x.as_table()) {
            for (k, val) in scripts {
                if let Some(v) = val.as_str() {
                    meta.entry_points.push(format!("{k} -> {v}"));
                }
            }
        }
    }
    let mut deps: Vec<String> = Vec::new();
    if let Some(list) = v
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for item in list {
            if let Some(s) = item.as_str() {
                let name = s
                    .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
                    .next()
                    .unwrap_or(s)
                    .to_string();
                deps.push(name);
            }
        }
    }
    deps.sort();
    deps.dedup();
    let group = DepGroup {
        ecosystem: "PyPI".into(),
        deps,
    };
    Some((meta, group))
}

fn parse_requirements(path: &Path) -> Option<DepGroup> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut deps = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        let name = line
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != '.')
            .next()
            .unwrap_or(line)
            .to_string();
        if !name.is_empty() {
            deps.push(name);
        }
    }
    deps.sort();
    deps.dedup();
    Some(DepGroup {
        ecosystem: "PyPI".into(),
        deps,
    })
}

fn parse_go_mod(path: &Path) -> Option<(ProjectMeta, DepGroup)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut meta = ProjectMeta {
        project_type: "Go application/library".into(),
        package_manager: "go modules".into(),
        language: "Go".into(),
        ..ProjectMeta::default()
    };
    let mut deps: Vec<String> = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("module ") {
            meta.name = rest.trim().to_string();
        } else if t == "require (" {
            in_require = true;
        } else if t == ")" {
            in_require = false;
        } else if let Some(rest) = t.strip_prefix("require ") {
            let rest = rest.trim();
            let name = rest.split_whitespace().next().unwrap_or(rest).to_string();
            deps.push(name);
        } else if in_require {
            let name = t.split_whitespace().next().unwrap_or(t).to_string();
            if !name.is_empty() && !name.starts_with("//") {
                deps.push(name);
            }
        }
    }
    if path
        .parent()
        .map(|p| p.join("main.go").is_file())
        .unwrap_or(false)
    {
        meta.entry_points.push("main.go".into());
    }
    meta.run_command = Some("go run .".into());
    meta.build_command = Some("go build".into());
    deps.sort();
    deps.dedup();
    let group = DepGroup {
        ecosystem: "Go modules".into(),
        deps,
    };
    Some((meta, group))
}

fn parse_gemfile(path: &Path) -> Option<(ProjectMeta, DepGroup)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut meta = ProjectMeta {
        project_type: "Ruby application".into(),
        package_manager: "bundler".into(),
        language: "Ruby".into(),
        ..ProjectMeta::default()
    };
    let mut deps = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("gem ") {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            if !name.is_empty() {
                deps.push(name);
            }
        }
    }
    meta.run_command = Some("bundle exec".into());
    deps.sort();
    deps.dedup();
    Some((
        meta,
        DepGroup {
            ecosystem: "RubyGems".into(),
            deps,
        },
    ))
}

fn parse_composer(path: &Path) -> Option<(ProjectMeta, DepGroup)> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut meta = ProjectMeta {
        project_type: "PHP application".into(),
        package_manager: "composer".into(),
        language: "PHP".into(),
        ..ProjectMeta::default()
    };
    meta.name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").into();
    let mut deps = Vec::new();
    for key in ["require", "require-dev"] {
        if let Some(map) = v.get(key).and_then(|x| x.as_object()) {
            for name in map.keys() {
                deps.push(name.clone());
            }
        }
    }
    deps.sort();
    deps.dedup();
    Some((
        meta,
        DepGroup {
            ecosystem: "Packagist".into(),
            deps,
        },
    ))
}

fn infer_framework(meta: &mut ProjectMeta, deps: &[DepGroup]) {
    if !meta.framework.is_empty() {
        return;
    }
    let known: &[(&str, &[&str])] = &[
        ("Express.js", &["express"]),
        ("Fastify", &["fastify"]),
        ("NestJS", &["@nestjs/core", "nest"]),
        ("Next.js", &["next"]),
        ("Nuxt", &["nuxt"]),
        ("Koa", &["koa"]),
        ("Hono", &["hono"]),
        ("React", &["react"]),
        ("Vue", &["vue"]),
        ("Svelte", &["svelte"]),
        ("Angular", &["@angular/core"]),
        ("Astro", &["astro"]),
        ("Rocket (Rust)", &["rocket"]),
        ("Axum (Rust)", &["axum"]),
        ("Actix-web (Rust)", &["actix-web"]),
        ("Tokio (Rust)", &["tokio"]),
        ("Django", &["django"]),
        ("Flask", &["flask"]),
        ("FastAPI", &["fastapi"]),
        ("Gin (Go)", &["gin-gonic/gin"]),
        ("Echo (Go)", &["labstack/echo"]),
        ("Chi (Go)", &["go-chi/chi"]),
        ("Rails", &["rails"]),
        ("Symfony (PHP)", &["symfony/console"]),
        ("Laravel (PHP)", &["laravel/framework"]),
    ];
    let mut all: Vec<&str> = Vec::new();
    for g in deps {
        for d in &g.deps {
            let low = d.to_lowercase();
            for (frame, needles) in known {
                if needles.iter().any(|n| {
                    low == *n || low.starts_with(&format!("{n} ")) || low.contains(&format!("/{n}"))
                }) {
                    all.push(frame);
                }
            }
        }
    }
    all.sort();
    all.dedup();
    meta.framework = all.join(", ");
}
