use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectMeta {
    pub name: String,
    pub project_type: String,
    pub language: String,
    pub framework: String,
    pub package_manager: String,
    pub entry_points: Vec<String>,
    pub run_command: Option<String>,
    pub build_command: Option<String>,
    pub scripts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub kind: String,
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub path: String,
    pub language: Option<String>,
    pub loc: usize,
    pub symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentHeading {
    pub path: String,
    pub heading_path: Vec<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DepGroup {
    pub ecosystem: String,
    pub deps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DirStat {
    pub files: usize,
    pub loc: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ProjectMap {
    pub root: String,
    pub meta: ProjectMeta,
    /// relative path -> aggregate stats
    pub dir_stats: BTreeMap<String, DirStat>,
    pub total_files: usize,
    pub total_loc: usize,
    pub languages: BTreeMap<String, usize>, // lang -> file count
    pub key_files: Vec<FileInfo>,
    pub document_headings: Vec<DocumentHeading>,
    pub deps: Vec<DepGroup>,
    pub guardrails: Vec<String>,
    pub profile_used: MapProfile,
    pub excluded_paths: Vec<String>,
    pub generated_at: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub enum MapProfile {
    #[default]
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "mini")]
    Mini,
}

impl MapProfile {
    pub fn is_mini(&self) -> bool {
        matches!(self, MapProfile::Mini)
    }
}

/// auto：依專案規模選（小專案用迷你地圖，避免負效益）
pub fn resolve_profile(
    total_loc: usize,
    _total_files: usize,
    flag: Option<MapProfile>,
) -> MapProfile {
    match flag {
        Some(MapProfile::Mini) | Some(MapProfile::Full) => flag.unwrap(),
        _ => {
            // 讀完整個專案 <300 行時，地圖反而比直接讀貴 -> 用迷你地圖
            if total_loc < 300 {
                MapProfile::Mini
            } else {
                MapProfile::Full
            }
        }
    }
}
