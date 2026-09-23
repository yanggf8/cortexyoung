//! Tree-sitter symbol extraction (deterministic, fact-only)
use crate::model::Symbol;
use tree_sitter::{Language, Parser};

fn lang_for(lang: &str) -> Option<Language> {
    let l: Language = match lang {
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        _ => return None,
    };
    Some(l)
}

/// 本地是否有該語言的 tree-sitter grammar。沒有的語言（java/ruby/php/kotlin…）
/// 檔案照樣進 LOC/樹/key files，只是**沒有符號**——地圖必須說出這件事，
/// 否則「檔案在、符號空」看起來像「這檔沒結構」。有符號的那條路是 `navigate --cort`。
pub fn has_grammar(lang: &str) -> bool {
    lang_for(lang).is_some()
}

fn interesting_kinds(lang: &str) -> &'static [&'static str] {
    match lang {
        "javascript" => &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "lexical_declaration",
        ],
        "typescript" => &[
            "function_declaration",
            "class_declaration",
            "abstract_class_declaration",
            "method_definition",
            "interface_declaration",
            "type_alias_declaration",
            "enum_declaration",
            "lexical_declaration",
        ],
        "python" => &[
            "function_definition",
            "class_definition",
            "decorated_definition",
        ],
        "rust" => &[
            "function_item",
            "struct_item",
            "enum_item",
            "trait_item",
            "impl_item",
            "mod_item",
            "type_item",
            "const_item",
            "static_item",
            "macro_definition",
        ],
        "go" => &[
            "function_declaration",
            "method_declaration",
            "type_declaration",
        ],
        "c" => &[
            "function_definition",
            "struct_specifier",
            "enum_specifier",
            "union_specifier",
        ],
        "cpp" => &[
            "function_definition",
            "class_specifier",
            "struct_specifier",
            "enum_specifier",
            "namespace_definition",
            "union_specifier",
        ],
        _ => &[],
    }
}

fn name_of(node: tree_sitter::Node, src: &str, kind: &str, lang: &str) -> Option<String> {
    if kind == "impl_item" {
        if let Some(t) = node.child_by_field_name("type") {
            return Some(format!("impl {}", t.utf8_text(src.as_bytes()).ok()?));
        }
        return Some("impl".to_string());
    }
    // C/C++ function names live inside the declarator chain
    if (lang == "c" || lang == "cpp") && kind == "function_definition" {
        let mut n = node.child_by_field_name("declarator");
        let mut hops = 0;
        while let Some(cur) = n {
            if let Ok(t) = cur.utf8_text(src.as_bytes()) {
                let clean = t.trim();
                if is_plain_name(clean) {
                    return Some(clean.to_string());
                }
            }
            n = cur
                .child_by_field_name("declarator")
                .or_else(|| cur.child_by_field_name("name"));
            hops += 1;
            if hops > 6 {
                break;
            }
        }
    }
    // Go: type_declaration -> type_spec -> name
    if lang == "go" && kind == "type_declaration" {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            if child.kind() == "type_spec" {
                if let Some(n) = child.child_by_field_name("name") {
                    return n.utf8_text(src.as_bytes()).ok().map(|s| s.to_string());
                }
            }
        }
    }
    // Python decorated_definition: delegate to inner function/class name
    if lang == "python" && kind == "decorated_definition" {
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            let ck = child.kind();
            if ck == "function_definition" || ck == "class_definition" {
                return name_of(child, src, ck, lang);
            }
        }
    }
    if let Some(n) = node.child_by_field_name("name") {
        return n.utf8_text(src.as_bytes()).ok().map(|s| s.to_string());
    }
    // fallback: first identifier-ish named child (shallow)
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let ckind = child.kind();
        if ckind.contains("identifier") || ckind == "type_identifier" || ckind == "field_identifier"
        {
            return child.utf8_text(src.as_bytes()).ok().map(|s| s.to_string());
        }
    }
    // lexical_declaration: look into variable_declarator
    let mut c2 = node.walk();
    for child in node.named_children(&mut c2) {
        if child.kind() == "variable_declarator" {
            if let Some(n) = child.child_by_field_name("name") {
                return n.utf8_text(src.as_bytes()).ok().map(|s| s.to_string());
            }
        }
    }
    None
}

fn is_plain_name(t: &str) -> bool {
    !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn kind_label(lang: &str, kind: &str) -> String {
    match (lang, kind) {
        (_, "function_declaration") | (_, "function_definition") | (_, "function_item") => {
            "fn".into()
        }
        (_, "class_declaration")
        | (_, "class_specifier")
        | (_, "class_definition")
        | (_, "abstract_class_declaration") => "class".into(),
        (_, "method_definition") | (_, "method_declaration") => "method".into(),
        (_, "interface_declaration") => "interface".into(),
        (_, "type_alias_declaration") | (_, "type_item") | (_, "type_declaration") => "type".into(),
        (_, "enum_declaration") | (_, "enum_item") | (_, "enum_specifier") => "enum".into(),
        (_, "struct_item") | (_, "struct_specifier") => "struct".into(),
        (_, "trait_item") => "trait".into(),
        (_, "impl_item") => "impl".into(),
        (_, "mod_item") => "mod".into(),
        (_, "const_item") | (_, "lexical_declaration") | (_, "variable_declaration") => {
            "const".into()
        }
        (_, "static_item") => "static".into(),
        (_, "macro_definition") => "macro".into(),
        (_, "namespace_definition") => "namespace".into(),
        (_, "union_specifier") => "union".into(),
        ("python", "decorated_definition") => "decorated".into(),
        (_, k) => k.into(),
    }
}

const FUNCTION_LIKE: &[&str] = &[
    "function_declaration",
    "arrow_function",
    "function_expression",
    "method_definition",
    "function_item",
    "function_definition",
    "function_signature",
    "lambda",
    "closure_expression",
    "func_literal",
];

fn is_nested(node: tree_sitter::Node) -> bool {
    let mut cur = node.parent();
    let mut hops = 0;
    while let Some(p) = cur {
        if FUNCTION_LIKE.contains(&p.kind()) {
            return true;
        }
        cur = p.parent();
        hops += 1;
        if hops > 30 {
            break;
        }
    }
    false
}

pub fn extract_symbols(lang: &str, source: &str) -> Vec<Symbol> {
    let Some(language) = lang_for(lang) else {
        return vec![];
    };
    let kinds = interesting_kinds(lang);
    if kinds.is_empty() {
        return vec![];
    }
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return vec![];
    }
    let Some(tree) = parser.parse(source, None) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        let is_dup_decorated = node
            .parent()
            .map(|p| p.kind() == "decorated_definition")
            .unwrap_or(false);
        if !is_dup_decorated && kinds.contains(&kind) && !is_nested(node) {
            if let Some(name) = name_of(node, source, kind, lang) {
                let label = kind_label(lang, kind);
                let line = node.start_position().row + 1;
                out.push(Symbol {
                    kind: label,
                    name,
                    line,
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    out.sort_by_key(|a| a.line);
    out.truncate(120);
    out
}
