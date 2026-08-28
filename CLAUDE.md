# CLAUDE.md

This repo builds `cort`, an offline code-intelligence CLI over `ast-grep` + SQLite.

Routing for day-to-day work is in `skills/ast-grep/SKILL.md`. `ast-grep` is the only parser: never add an in-process parser, never call the `sg` binary, and never loosen the `0.45.2` version pin.

Run `cargo test` in `rust/` before every commit (`bash tests/install-smoke.sh` too when touching `install.sh`). If `cort` or `ast-grep` is missing, point the user to `./install.sh` — do not install automatically.
