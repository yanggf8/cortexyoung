---
name: claudecat
description: Run ClaudeCat (pure Rust CLI v2) to map and navigate a project, quantify exploration cost, integrate with cortexyoung/cort, and run the daily cort-audit. Use when the user asks to scan/map a project, find where a symbol lives, check cort index health, run the daily audit and write findings/metrics into CORT-AUDIT.md, update the claudecat auto blocks, or invokes /claudecat.
---

# ClaudeCat — Project Map & cort Integration

Pure Rust CLI. Run via Bash; binary is on PATH as `claudecat`. All subcommands default to cwd (`--root` to override).

## Commands

```bash
claudecat scan                  # 掃描專案，輸出導航地圖（--format text|markdown|json，--map mini|auto|full）
claudecat explore --json        # 量化探索成本：無地圖 vs 地圖的 token 粗估
claudecat navigate "auth"       # 從一句話找目的地符號/檔案；--cort 優先用 cort 全量索引（需先 cort index）
claudecat usage --json           # 查看本機導航命中統計（不含原始查詢）
claudecat update --dry-run      # 更新 claudecat 自動區塊；--dry-run 只顯示差異
claudecat cort-status           # cort 索引狀態：新鮮度、chunk、relationships
claudecat doctor                # 體檢追蹤循環；--install 一鍵部署 crontab（冪等）
claudecat cort-audit --track CORT-AUDIT.md   # 每日審計 + 寫長期指標表（原子、同日更新）
claudecat findings - <<'EOF'    # 把每日分析發現寫進文件發現區塊（--file 預設 CORT-AUDIT.md）
...markdown findings...
EOF
```

## Where output goes

- `update` writes **only** the `claudecat:auto` / `guardrails` / `map-pointer` blocks in the rules file (AGENTS.md / CLAUDE.md symlink) — never touches hand-written rules.
- The generated project map goes to `~/.local/share/claudecat/<project-id>/map.md` (`CLAUDECAT_DATA_DIR` overrides); `update` prints the `map ->` path. Maps do not go into git.

## Daily audit workflow

1. `claudecat cort-audit --track CORT-AUDIT.md` — appends/updates today's row in the long-term metrics table.
2. Read the full report (index health, coverage gaps, hook census, harness hit rates).
3. `claudecat findings -` with a curated markdown summary (date, key deltas, rule decisions) — replaces that day's findings block atomically, preserving everything outside the block.

## Gone in v2 (do not use)

- No `sync` / `login` / Turso cloud — cross-machine state is per-machine, DBs are independent (kimi denominators differ across hosts by design).
- No MCP server — never re-register one; the CLI is the only interface.
- No `npm install -g claudecat` — install with `cargo install --path <repo>` from the Rust checkout; `~/.local/bin/claudecat` typically symlinks to the repo's `target/release/claudecat`.

## Markdown navigation sidecar

`navigate` also indexes gitignore-aware Markdown headings independently from code symbols.
When a document matches, follow the reported `cort read <file> --start N --end N` route;
run it from the target project root. A document heading never becomes a `cort impact` symbol.
Navigation usage is recorded locally without raw query text; set `CLAUDECAT_NO_USAGE=1` to
disable it. Use `claudecat usage --json` to inspect whether the sidecar is helping.
