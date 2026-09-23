//! Project map rendering: markdown section for CLAUDE.md / human output
use crate::model::{MapProfile, ProjectMap};
use crate::walk::tree_lines;

pub fn render_markdown(map: &ProjectMap) -> String {
    render_with_profile(map, MapProfile::Full)
}

pub fn render_with_profile(map: &ProjectMap, profile: MapProfile) -> String {
    if profile == MapProfile::Mini {
        return render_mini(map);
    }
    render_full(map)
}

fn render_full(map: &ProjectMap) -> String {
    let mut s = String::new();
    s.push_str("## Project Map (auto-maintained by claudecat)\n");
    s.push_str(&format!("- **Root**: `{}`\n", map.root));
    if !map.meta.project_type.is_empty() {
        s.push_str(&format!("- **Type**: {}\n", map.meta.project_type));
    }
    if !map.meta.language.is_empty() {
        s.push_str(&format!("- **Language**: {}\n", map.meta.language));
    }
    if !map.meta.framework.is_empty() {
        s.push_str(&format!("- **Framework**: {}\n", map.meta.framework));
    }
    if !map.meta.package_manager.is_empty() {
        s.push_str(&format!(
            "- **Package manager**: {}\n",
            map.meta.package_manager
        ));
    }
    if !map.meta.entry_points.is_empty() {
        s.push_str(&format!(
            "- **Entry points**: {}\n",
            map.meta.entry_points.join(", ")
        ));
    }
    if let Some(c) = &map.meta.run_command {
        s.push_str(&format!("- **Run**: `{}`\n", c));
    }
    if let Some(c) = &map.meta.build_command {
        s.push_str(&format!("- **Build**: `{}`\n", c));
    }
    let langs: Vec<String> = map
        .languages
        .iter()
        .map(|(k, v)| format!("{k}({v})"))
        .collect();
    s.push_str(&format!(
        "- **Scale**: {} files, {} LOC [{}]\n",
        map.total_files,
        map.total_loc,
        langs.join(", ")
    ));
    if !map.meta.scripts.is_empty() {
        let scripts: Vec<String> = map
            .meta
            .scripts
            .iter()
            .map(|(k, v)| format!("`{}`: {}", k, v))
            .collect();
        s.push_str(&format!("- **Scripts**: {}\n", scripts.join(" · ")));
    }

    s.push_str("\n### Directory structure\n```\n");
    for line in tree_lines(&map.dir_stats, 2, 40, map.total_loc) {
        s.push_str(&line);
        s.push('\n');
    }
    s.push_str("```\n");

    if !map.key_files.is_empty() {
        s.push_str("\n### Key files & symbols\n");
        for f in &map.key_files {
            let syms = if f.symbols.is_empty() {
                String::new()
            } else {
                let names: Vec<String> = f
                    .symbols
                    .iter()
                    .map(|sym| format!("{}:{} {}", sym.line, sym.kind, sym.name))
                    .collect();
                format!("  — {}", names.join("; "))
            };
            s.push_str(&format!(
                "- `{}` ({} LOC, {}){}\n",
                f.path,
                f.loc,
                f.language.clone().unwrap_or_else(|| "?".into()),
                syms
            ));
        }
        // 沒有本地 grammar 的語言（cort 端有：Java 等）——空符號不是「沒結構」，要說清楚
        let no_ast: std::collections::BTreeSet<&str> = map
            .key_files
            .iter()
            .filter(|f| f.symbols.is_empty())
            .filter_map(|f| f.language.as_deref())
            .filter(|l| !crate::symbols::has_grammar(l))
            .collect();
        if !no_ast.is_empty() {
            s.push_str(&format!(
                "\n> {} 檔無本地 AST（claudecat 未內建該 grammar）——符號走 `claudecat navigate --cort` 或 `cort struct`。\n",
                no_ast.into_iter().collect::<Vec<_>>().join(" / ")
            ));
        }
    }

    if !map.deps.is_empty() {
        s.push_str("\n### Dependencies (declared)\n");
        for g in &map.deps {
            if g.deps.is_empty() {
                continue;
            }
            let take = g.deps.len().min(30);
            s.push_str(&format!(
                "- `{}`: {}\n",
                g.ecosystem,
                g.deps[..take].join(", ")
            ));
            if g.deps.len() > take {
                s.push_str(&format!("  … +{} more\n", g.deps.len() - take));
            }
        }
    }

    if !map.guardrails.is_empty() {
        s.push_str("\n### 技術決策 / Guardrails（開發者維護，claudecat 永不覆寫）\n");
        for g in &map.guardrails {
            s.push_str(&format!("- {g}\n"));
        }
    }

    if !map.excluded_paths.is_empty() {
        s.push_str(&format!(
            "\n**Excluded dirs**: {}\n",
            map.excluded_paths.join(", ")
        ));
    }
    s.push_str(&format!("\n*Generated at {} by claudecat* — facts from manifests + AST; framework/entry may be inferred from deps/paths where manifest lacks them.\n", map.generated_at));
    s
}

fn render_mini(map: &ProjectMap) -> String {
    let mut s = String::new();
    s.push_str("## Project Map (auto-maintained by claudecat) — Mini\n");
    let bits: Vec<String> = vec![
        Some(map.meta.project_type.clone()).filter(|x| !x.is_empty()),
        Some(map.meta.language.clone()).filter(|x| !x.is_empty()),
        Some(map.meta.framework.clone()).filter(|x| !x.is_empty()),
        Some(map.meta.package_manager.clone()).filter(|x| !x.is_empty()),
    ]
    .into_iter()
    .flatten()
    .collect();
    s.push_str(&format!("- **About**: {}\n", bits.join(" · ")));
    if !map.meta.entry_points.is_empty() {
        s.push_str(&format!(
            "- **Entry points**: {}\n",
            map.meta.entry_points.join(", ")
        ));
    }
    if let Some(c) = &map.meta.run_command {
        s.push_str(&format!("- **Run**: `{}`\n", c));
    }
    if let Some(c) = &map.meta.build_command {
        s.push_str(&format!("- **Build**: `{}`\n", c));
    }
    let top_ = crate::walk::tree_lines(&map.dir_stats, 1, 6, map.total_loc);
    let top: String = top_
        .iter()
        .filter(|l| !l.starts_with("Total:"))
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if !top.is_empty() {
        s.push_str(&format!("- **Structure**: {}\n", top));
    }
    if !map.deps.is_empty() {
        let dep_bits: Vec<String> = map
            .deps
            .iter()
            .map(|g| {
                format!(
                    "{}: {}",
                    g.ecosystem,
                    g.deps
                        .iter()
                        .take(8)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect();
        s.push_str(&format!("- **Deps**: {}\n", dep_bits.join("; ")));
    }
    if !map.guardrails.is_empty() {
        s.push_str("- **Guardrails**: ");
        s.push_str(&map.guardrails.join(" | "));
        s.push('\n');
    }
    s.push_str(&format!("\n*Mini map — {}\n", map.generated_at));
    s
}
