# AGENTS.md

本檔提供 Claude Code / Codex 在本 repo 工作時的指引；CLAUDE.md 是本檔的 symlink。

- 每日 cort-audit 分析的**發現**寫在 [CORT-AUDIT.md](CORT-AUDIT.md) 的「每日分析發現」區，
  長期指標表在同一份檔。本檔只留規則，文件放文件。

<!-- claudecat:guardrails:begin -->
<!-- 技術決策 / Guardrails：每行一條，例如 `2D tilemap + Macroquad（禁 Python/3D）`、`插件一律裝在 agent harness 內`。claudecat 只在此區不存在時建立，之後永不覆寫。 -->
<!-- claudecat:guardrails:end -->

<!-- claudecat:map-pointer:begin -->
- 專案地圖（Project Map）不在本檔：`claudecat update` 會把它寫到各機器的 `~/.local/share/claudecat/<project-id>/map.md`（`CLAUDECAT_DATA_DIR` 可覆蓋），並在輸出印出 `map ->` 的確切路徑。本檔只放手寫規則，產生的地圖不進 git。本區由 claudecat 播種一次，之後永不改寫。
<!-- claudecat:map-pointer:end -->
