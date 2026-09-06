# AGENTS.md

Instructions for any coding agent working in this repository (Codex reads `AGENTS.md`; Claude Code reads `CLAUDE.md`, which is a symlink to this file so the two can never drift).

This repo builds `cort`, an offline code-intelligence CLI over `ast-grep` + SQLite.

**The product's long-term goal is one sentence, and it is a measurement rather than a sentiment: make
the caller-set enumeration an agent already performs — and often gets wrong — cheap, checkable edge by
edge, and able to say whether the set is complete.** The third clause was promoted from a property of
the second to a goal in its own right on 2026-09-01, because the two are answered by different
machinery and one of them is done: `verify-impact` grades a printed edge against its call site
(soundness, "is this row real"), while `impact --coverage` is the only thing that speaks to
completeness ("is any caller missing"), and completeness is where the tool is currently weakest — see
the receiver-gate and 62-of-63 numbers below. A caller set nobody can bound is not evidence, however
cheap it was to produce. Two scope facts to state rather than imply: the screen answers for
**callers** only (`impact` emits `dependents`; there is no callee/`dependencies` direction anywhere in
the product as of 2026-09-01), and it is a text-and-index screen, so it can be honest about what it
did not read but can never be a compiler.

Do not justify graph work by saying users ask relationship questions. They do not: on the
surviving local corpus (`cort-evals demand`, `docs/2026-08-31-demand-recheck.md`) 1,214 genuine user
instructions held **one** relational question (0.08%), and 4-7 (0.33-0.58%) were instructions that
cannot be done correctly without a call-site set - all of them on the delete / refactor / review path.
The same corpus shows 42% of what arrives as a "user message" is a pasted agent report being fact-checked,
which is the real surface: the agent does this work unprompted, gets multi-hop answers wrong in 6 of 10
cells (`evals/runs/2026-08-30-graph{,-sample2}/`), and is then asked to prove it. Cost per use is already
settled (7.7x smaller tool payload — ~6.7x since schema v4 added two columns to the lean row — at ~4x
fewer turns, same venue, same task set; README's cost section carries both figures). **Checkability is the
open half.** Its first piece landed on 2026-08-31 (schema v4): `relationships` stores `call_site_line`
and `call_form`, `impact` prints `@<line> <form>` beside each dependent's definition line, and
`cort-evals verify-impact` grades an edge against that single line (117/117 dependents on the 5 cct
chains, 64/64 on 4 chains in this repo). The rest of that half is still open and it is the *other*
direction of the same claim: `enumeration_may_be_incomplete` now has two causes and no more (a named
gap row, or a file the screen never read -- `unparsed` became advisory on 2026-08-31, coverage-v2, after
two chunk-less files in this repo were flipping all 60 sampled seeds), and skill + README + the report's
own `reading` field say what `false` does and does not entitle anyone to conclude. Still open: the
receiver gate attaches 9 of 4,833 receiver call sites at `a0269cda` and 12 of 5,843 at `dbc971f7`
(was 12 of 5,212 at `d4637150` -- this line moves with the tree, quote it with its commit; all
correct in every graded run; the refusals are where recall still leaks, each one a `--coverage`
row -- `cort-evals recall-exp` re-derives the population, so quote it with its commit), and on a
hub-dense venue like cct the boolean is still true
for 62 of 63 sampled seeds -- which is why the instruction is *read the rows*, not *watch the flag*. A
change that makes an answer cheaper to verify is on the main line; a feature that only makes answers
more numerous is not.

**The routing rule has exactly one home: `rust/src/hook.rs`.** It is deployed as a `PreToolUse`
hook by `install.sh` in the same run as the skill, so while working in this repo your own
`grep`/`rg` will sometimes come back with a `cort impact` suggestion attached -- that is the
product talking, and it is the retrospective half of the routing the skill's prose could not
carry (409 searches in skill-bearing sessions, zero `cort` calls). Be precise about which half is
singular: **parsing is per-harness and plural, the verdict is singular.** Two spellings arrive in a
*hook payload* and each gets its own constructor in `hook.rs`: a shell line (`search_from_shell` --
Claude Code, Codex and Grok all send `tool_input.command` as a string) and Kimi's structured `Grep`
fields. Codex's `["bash","-lc",…]` is a third spelling but not a third one there: it is the *rollout
transcript* dialect, which only `cort-evals hook-probe` reads, so its extraction lives in
`evals/src/hook.rs` and hands the recovered script to `search_from_shell`. All of them build a
`Search` and hand it to the one `judge`. A second copy of a *parser* is just code; a second copy of
the *decision* makes `hook-probe`'s calibration describe something other than what ships, which is
the only thing that number is for. So `hook-probe` replays `judge` itself, and never reimplements it
-- a hand-rolled approximation of the rule was tried on 2026-09-02 and over-counted its own corpus
by 48% and 4x on the two surfaces (`docs/2026-09-02-hook-wiring-correction.md` §15, §16).

**A parser may be plural; the table may not.** `cort hook-install` owns the settings merge for the
same reason a `jq` pipeline would not -- preserving other people's hooks, collapsing duplicates, and
refusing a file it cannot parse are logic, and logic needs tests -- one module per dialect
(`settings.rs` JSON, `settings_toml.rs` Codex, `settings_kimi.rs` Kimi). It also owns **which
harnesses ship, which file each reads, which dialect it speaks, and which subcommand each event
runs**: that is `HOOK_TARGETS` in `main.rs`, reached through `hook-install --all`, and `install.sh`
holds none of it. It used to hold all four, restated on each of six calls, and bash's copy was the
one that shipped -- so the binary's own defaults were never exercised and had rotted into answering
`--status --format kimi` about `~/.claude/settings.json`
(`docs/2026-09-03-installer-dedup-and-attribution.md` §2, §3). The one fact only the installer has
is which binary the harness should run: the installed layout puts a shim in front of the real
executable and `--check` verifies the shim, so `--command-prefix` is required and never defaults to
`current_exe()`. `--all --lean` speaks TSV **with no empty field ever** -- tab is an IFS whitespace
character, so `read` collapses runs of tabs and drops the empty one between them. Recognition of our
own entry is a token test, never a suffix test -- anchoring it to the end of the command line is
what let `--status` report `wired: false` on a machine where the hook was firing, twice
(`docs/2026-09-02-hook-wiring-correction.md`).

**The name in a transcript is not the name in a payload.** Codex's rollouts call its shell tool
`exec_command`; its `PreToolUse` payload calls the same tool `Bash`, captured live on 2026-09-03
(`tool_name: "Bash"`, `tool_input.command: "ls"`, alongside `model`, `cwd` and `transcript_path`).
So `matcher = "Bash"` is correct and always was -- that is what §12's "Codex's payload is Claude
Code's byte for byte" covers, values and not just field names. Reading the rollout name and changing
the matcher to `exec_command` broke a hook that worked, and then four hypotheses were spent chasing
a symptom that change had created (the matcher's value, stale trust, the command's shape, the
working directory). **Judge a matcher only from an intercepted payload, never from a transcript.**

That normalisation is a rule and not a special case: a live run on 2026-09-03 16:48 showed Codex
firing the `PostToolUse` entry on a *file edit* -- so it renames its edit tool into Claude Code's
vocabulary the same way it renames `exec_command`, and `apply_patch` needs no matcher of its own.
The same run showed the alternation `Bash|Edit|Write|MultiEdit|NotebookEdit` matching a `Bash` call,
so Codex compiles a matcher as a regex, which nothing before had established (`"Bash"` was the only
value ever seen firing, and the one alternative tried could not have matched on either reading).
Neither fact came from an intercepted payload: the edit was identified by which hook *stayed
silent*. `Suggest`'s matcher is the bare `Bash`, so a fire on `Refresh` alone is a fire on something
that is not Bash. **Two entries with deliberately different matcher widths are an instrument, and
the narrow one is the control** -- worth keeping in mind before anyone tidies them into agreement.

**`--status` reports that a state exists, not that it is still current, and that has now misled
three times.** Twice it called a firing hook `wired: false` (a suffix test anchored to the end of
the command line, `docs/2026-09-02-hook-wiring-correction.md`); on 2026-09-03 it called
`trusted=true` an entry `install.sh` had just rewritten. The field is not miscomputed -- `trusted_at`
only ever asks whether a `trusted_hash` exists at that `gi:hi`, which its own doc comment says --
but that is a different question from the one a reader brings. Codex re-prompted for review on a
rewrite that changed **only the matcher** and left the command byte-identical, so `trusted_hash`
covers the entry's shape and the installer's notice at the moment of the rewrite is the
authoritative signal. Nothing after the fact can verify a trust; only a live run can
(`docs/2026-09-03-installer-dedup-and-attribution.md` §12, §13).

That interception has to be able to prove who called it. One round was lost to a `harness=codex`
row this session had written itself: all five in the local database were hand-fed self-tests, so the
"first real fire" two hypotheses rested on never happened. The probe that finally settled it logged
its parent's command line. **A figure that cannot say what produced it is not evidence** -- the same
rule as the machine stamp below, one level down: there it cannot name its machine, here it cannot
name its process. `docs/2026-09-03-installer-dedup-and-attribution.md` §7.

**A number that cannot say which machine produced it is not comparable to anything.** `usage.db`
stamps `MACHINE_ID` once and never rewrites it, so a database carried to a second machine keeps
naming its origin and the report can say `mixed`; `cort usage`'s lean header and every `cort-evals`
report carry the id and its source. This is not hypothetical bookkeeping -- on 2026-09-03 a table of
417 Codex fires was reconciled against this machine's 2 before anyone noticed they were different
computers. Rows carry no machine of their own, so once two machines have written to one database
nothing can separate them again; the report's only honest move is to refuse to be quiet about it.
**Harness is which program ran, model is who answered, and machine is where** -- three dimensions,
and the harness total stays whole regardless of the other two (`hook_models_at` is a second lens,
never a split of the first).

**The hook never blocks -- except on Kimi, where it can only block.** Kimi's `PreToolUse` keeps only
results whose `action` is `block` and discards every allow-shaped one before the model sees it, so a
suggestion there arrives as a deny or not at all. That exception is bounded and must stay bounded:
once per symbol per session, then yield, and `no_other_harness_ever_receives_a_deny` is a test.
Whether the deny actually changes what the agent does is still two runs and one uptake -- do not
quote it as established (§16).

**The repo is pure Rust. No JavaScript, TypeScript, Python or other scripting language may exist as executable code** — not as a product entry point, not as tooling, not as tests. The eval harness that used to be six `.mjs` files was ported into the `evals/` crate for exactly this reason. Bash stays only where the platform requires it (`install.sh`, `tests/install-smoke.sh`); it is not a place to put logic. If a task seems to need a script, add a Rust subcommand to `evals/` or `rust/` instead.

**No absolute paths from any developer's machine, and no Node-installed toolchain paths, anywhere in
the repo — including test fixtures and fallbacks.** `ast-grep` is provisioned by `install.sh` from the
pinned native release asset (or `cargo install`); a test that cannot find it prints `SKIP:` instead of
reaching for a host-specific binary.

**A `skills/<name>/SKILL.md` is deployed byte-for-byte.** Nothing — not an ownership marker, not a
comment, not a banner — may be inserted into it, because two third-party parsers own that format and
the frontmatter key set is closed. `install.sh` claims ownership in `.cortexyoung-managed` beside the
file, recording the SHA-256 of the bytes it wrote; `rust/tests/skill_format.rs` gates the source shape
and `tests/install-smoke.sh` gates the deployed shape.

Everything executable is Rust except two files bash actually requires: `install.sh` and
`tests/install-smoke.sh`. The ast-grep test double is a Rust bin (`fake_ast_grep`, declared in
`rust/Cargo.toml` and covered by `rust/tests/fixture.rs`), always built so `cargo test` needs no
special feature and never installed — `tests/install-smoke.sh` asserts the payload ships nothing
but `cort` and the pack.

**Two hook events, and the second one repairs rather than reports.** `PreToolUse` on the search
tools suggests `cort impact`; `PostToolUse` on the edit tools runs `cort index --incremental`. The
second exists because the first could only ever *say* the index was behind and that was measured at
19 re-index runs against 2,700+ fires in 90 days -- and because `PreToolUse`'s staleness compares
git heads, so an edited-but-uncommitted tree read as fresh while its answers were already wrong.
`hook-refresh` never creates an index, gives up rather than wait on a busy database, and is silent
and exits 0 whatever happens. Both entries live in the same three files, told apart by their
subcommand rather than by position -- in Kimi's flat `[[hooks]]` array nothing else can tell them
apart. An install-then-uninstall cycle must hand every one of those files back byte for byte,
including comments that belong to other owners; that is a test, because it was a bug first
(`docs/2026-09-02-hook-wiring-correction.md` §17).

**Both staleness checks are one-sided, and they lean opposite ways.** The head comparison
(`main.rs`, what `hook-suggest` reads) misses an edited-but-uncommitted tree; the content
comparison (`staleness.rs`, the `index_is_stale` printed beside every `impact` row) missed
everything that moves HEAD *without* dirtying the tree -- `pull`, `checkout`, `rebase`, `reset`, a
teammate's commit -- because its candidate set was `git diff HEAD` and after a pull that diff is
empty. Fixed on 2026-09-03 by giving `git_candidates` the head the index was built from and
diffing against it too (`docs/2026-09-03-installer-dedup-and-attribution.md` §9). The half worth remembering is the second one: `index --incremental` shared
that candidate set, so it re-extracted nothing and then stamped the new head on anyway, which means
the `PostToolUse` repair hook was not merely failing to repair a pulled tree -- on the next edit it
*erased the one signal* saying repair was needed. Measured on this repo at `70e228f5..0d51e55b`:
`index_is_stale: false` over 17 changed files and a 5-commit-old graph. **A candidate set that
cannot be narrowed honestly must widen to everything**, never quietly to nothing, which is what
`narrowed: false` now means for a missing git *and* for a stored head git cannot resolve.

**CI's first gate is `rustfmt`, and a gate that fails is a gate that hides the ones behind it.** Both
jobs run fmt, then clippy with warnings as errors, then `cargo test --locked --all-targets`; a
rustfmt failure skips the rest, so the repo spent days reporting on every push while checking
nothing (`docs/2026-09-02-hook-wiring-correction.md` §18). Editing this repo through `sed`/`perl`
formats nothing, so run `cargo fmt --all` in **both** crates before committing, and run clippy too
-- the pass that finally ran caught a doc comment that had been separated from its function and was
silently documenting a different one. **Storage failures are returned, never panicked on.** A
transient `SQLITE_IOERR_FSYNC` on a CI runner killed eight tests through four `.expect()` calls in
`rust/src/db.rs`, and the same panic would break `hook-refresh`'s promise to be silent and exit 0 on
every edit for as long as a disk misbehaved.

## Navigation front door (claudecat, read-only)

To locate a symbol, a file, or who calls it, run `claudecat navigate --cort "<query>"` **first**.
It reads this repo's cort index read-only and prints a route (`file:symbol:line`), a compressed
content summary, and the `cort impact` / `cort context` commands for the deep-dive — caller-set
questions then start from `impact`, not from grep. Fall back to grep/ast-grep only when it
misses. The map section below is maintained by the same tool (`claudecat update`); it writes
nothing else, and `CLAUDE.md` stays a symlink to this file.

<!-- claudecat:auto:begin -->
## Project Map (auto-maintained by claudecat)
- **Root**: `/home/yanggf/a/cortexyoung`
- **Scale**: 148 files, 36244 LOC [config(76), rust(69), shell(2), typescript(1)]

### Directory structure
```
tests/ (1 files, 826 LOC)
Total: 72 files, 36244 LOC (code)
```

### Key files & symbols
- `rust/tests/cli.rs` (2322 LOC, rust)  — 11:const SAMPLE; 31:fn cort_bin; 35:fn make_project; 51:struct Run; 57:fn run_cort; 71:fn payload; 80:fn sandbox; 92:fn asking_a_command_for_help_explains_it_instead_of_running_it; 106:fn every_spelling_of_help_reaches_the_same_usage_and_none_of_them_is_an_error; 136:fn usage_documents_every_command_the_dispatcher_actually_knows; 159:fn an_unknown_command_is_still_a_failure_not_usage; 170:fn index_without_help_still_indexes_so_the_guard_did_not_swallow_the_command; 188:fn read_persists_a_fragment_and_recall_finds_it_through_fts; 216:fn unknown_format_is_a_structured_error_and_format_is_case_insensitive; 229:fn cort_cache_dir_is_honoured; 242:fn cli_canonicalizes_root_before_project_id_for; 273:fn missing_query_and_missing_symbol_are_structured_errors_not_panics; 287:fn type_method_end_to_end_cli_json_and_lean; 336:fn coverage_flows_through_the_cli_in_lean; 385:fn run_hook_suggest; 389:fn run_hook_suggest_with; 393:fn run_hook_suggest_full; 430:const FIRING_SEARCH; 433:fn the_hook_stays_quiet_when_the_db_exists_but_holds_no_index; 453:fn the_hook_speaks_once_the_project_is_actually_indexed; 478:fn the_usage_row_records_which_outcome_the_hook_reached; 517:fn git_in; 538:fn a_stale_index_is_disclosed_in_the_line_the_agent_reads; 603:fn an_index_predating_the_schema_is_reported_as_outdated_not_busy; 639:fn a_project_whose_directory_is_gone_can_still_be_deleted; 673:fn deleting_a_path_that_is_neither_a_directory_nor_a_row_still_fails; 698:fn a_usage_row_records_which_harness_fired_the_hook; 748:fn the_harness_is_taken_from_the_payload_not_from_the_flag_alone; 810:fn suppress_output_is_omitted_for_codex_and_kept_for_the_others; 855:fn run_hook_suggest_payload; 883:fn kimi_grep; 895:fn a_structured_grep_payload_is_read_the_same_as_a_shell_search; 920:fn kimi_denies_once_per_symbol_then_gets_out_of_the_way; 975:fn no_other_harness_ever_receives_a_deny; 1003:fn a_cache_directory_that_cannot_be_created_is_an_error_not_a_panic; 1044:fn hook_refresh_stays_silent_when_the_cache_is_unwritable; 1064:fn run_hook_refresh; 1093:fn run_hook_install; 1119:fn each_format_resolves_its_own_settings_file_when_settings_is_not_given; 1175:fn an_explicit_settings_path_still_overrides_the_format_default; 1203:fn run_hook_refresh_with; 1231:fn edit_payload; 1251:fn a_refresh_row_says_which_harness_wrote_it_and_is_never_summed_with_the_suggest_rows; 1355:fn a_hook_row_names_the_model_that_answered_and_never_invents_one; 1406:fn read_hook_rows; 1431:fn a_model_breakdown_never_splits_the_harness_total; 1504:fn the_lean_hook_report_has_six_non_empty_fields_on_every_line; 1598:fn the_hook_is_silent_when_the_index_holds_nothing_about_the_symbol; 1632:fn the_hook_still_fires_for_a_symbol_the_index_holds; 1654:fn the_hook_still_fires_after_the_definition_is_deleted_and_reindexed; 1697:fn git_in_fixture; 1715:fn refresh_outcomes; 1728:fn a_refresh_resolves_the_project_from_the_payload_not_the_shell; 1767:fn a_relative_path_in_the_payload_resolves_against_the_directory_the_hook_runs_in; 1804:fn a_relative_path_resolves_against_the_payload_cwd_before_the_process_cwd; 1840:fn a_refresh_with_no_path_in_the_payload_still_uses_the_working_directory; 1866:fn an_edited_path_outside_every_index_refuses_rather_than_repairing_cwd; 1896:fn a_schema_only_database_at_cwd_is_not_refreshed_into_an_index; 1929:fn projects_reports_extractor_drift_while_git_says_fresh; 1993:fn projects_reports_schema_drift_independently_of_the_extractor; 2036:fn an_unreadable_index_makes_the_verdict_unknown_not_drifted; 2073:fn drift_outranks_unreadable_and_each_count_stands_alone; 2106:fn a_drifted_index_whose_directory_is_gone_is_still_counted_as_drift; 2143:fn the_verdict_line_is_one_line_of_four_non_empty_fields; 2177:fn the_verdict_and_staleness_agree_about_an_incomplete_graph; 2214:fn the_refresh_hook_refuses_a_full_rebuild_and_says_so; 2284:fn a_foreground_incremental_still_rebuilds; 2315:fn the_refresh_hook_refuses_an_incomplete_graph_too; 2356:fn status_does_not_contradict_itself_about_the_extractor; 2393:fn the_refresh_hook_refuses_when_git_cannot_narrow_the_candidates; 2426:fn the_refresh_hook_does_not_migrate_the_schema
- `rust/src/main.rs` (1963 LOC, rust)  — 32:const KNOWN_COMMANDS; 48:fn usage_value; 73:fn wants_help; 78:fn map_index; 94:struct IdxWrap; 96:impl impl IdxWrap; 97:fn fmt; 102:impl impl IdxWrap; 103:fn sqlite_code; 111:struct CortWrap; 113:impl impl CortWrap; 114:fn fmt; 119:impl impl CortWrap; 120:fn sqlite_code; 125:fn unwrap_busy; 136:fn open_project_tracked; 162:fn open_project_unmigrated; 181:struct UsageEvent; 193:struct Emit; 199:fn usage_from_args; 219:fn fill_stale; 223:fn stored_omitted_len; 237:fn finish_record; 256:fn render_emit; 273:fn resolve_fmt; 278:fn parse_usize_flag; 284:fn parse_i64_flag; 290:fn parse_line_flag; 301:fn clap_fail; 312:fn content_mode_name; 320:fn cwd; 330:struct IndexArgs; 344:struct RootArgs; 356:struct HookSuggestArgs; 372:struct HookInstallArgs; 423:struct FormatOnlyArgs; 434:struct ProjectsArgs; 448:struct StructArgs; 467:struct ContextArgs; 485:struct ImpactArgs; 504:struct ReadArgs; 522:struct RecallArgs; 538:struct UsageArgs; 545:fn pin_bin; 551:fn dispatch; 600:fn hook_row; 611:fn hook_args; 617:fn model_of_payload; 650:fn harness_of; 679:fn search_of_payload; 714:fn gate_already_fired; 772:fn cmd_hook_refresh; 921:fn cmd_hook_suggest; 1089:const HOOK_GIT_BUDGET_MS; 1092:enum IndexState; 1114:fn git_head_quickly; 1125:fn probe_index; 1156:fn map_json_settings_err; 1167:fn map_toml_settings_err; 1187:enum SettingsFormat; 1193:fn settings_format; 1218:fn default_settings_path_for; 1255:const HOOK_TARGETS; 1269:fn hook_install_all; 1343:fn render_hook_entries_lean; 1399:fn status_of_entry; 1414:fn install_into; 1433:fn remove_from; 1443:fn cmd_hook_install; 1544:fn cmd_index; 1607:fn cmd_status; 1674:fn cmd_projects; 1755:fn cmd_delete; 1800:fn cmd_struct; 1843:fn cmd_context; 1879:fn cmd_impact; 1906:fn cmd_read; 1955:fn cmd_recall; 1986:fn cmd_usage; 2001:fn peek_format; 2024:fn main
- `rust/tests/usage.rs` (1262 LOC, rust)  — 16:const SENTINEL; 17:const NOW_MS; 18:const DAY_MS; 20:const SAMPLE; 36:fn cort_bin; 40:fn make_project; 56:struct Run; 62:fn run_cort; 76:fn payload; 85:fn sandbox; 95:fn usage_db; 99:fn project_db; 103:fn cache_names; 115:fn rec; 133:fn open_usage; 137:fn log_rows; 170:fn usage_files_contain; 184:fn insert_log; 216:fn seed_schema; 227:fn unindexed_cwd_status_creates_no_project_db; 243:fn help_creates_no_files_at_all; 268:fn usage_works_from_unindexed_cwd; 284:fn multi_project_rows_aggregate_in_one_report; 308:fn deleted_project_history_survives_delete_and_delete_does_not_recreate_db; 332:fn projects_stdout_baseline; 340:fn recorder_isolation_busy_leaves_command_stdout_and_exit_unchanged; 356:fn recorder_isolation_read_only_leaves_command_stdout_and_exit_unchanged; 373:fn recorder_isolation_corrupt_leaves_command_stdout_and_exit_unchanged; 384:fn recorder_isolation_sqlite_full_leaves_command_stdout_and_exit_unchanged; 437:fn recorder_isolation_mkdir_fail_leaves_command_stdout_and_exit_unchanged; 454:fn prune_failure_does_not_roll_back_the_insert; 501:fn usage_query_busy_or_corrupt_is_structured_error; 532:fn usage_days_boundaries_1_30_89_90_pass; 546:fn usage_days_0_negative_float_non_numeric_366_are_structured_errors; 563:fn retention_ts_before_cutoff_excluded_ts_equal_cutoff_kept; 585:fn unpruned_expired_rows_still_excluded_by_queries; 624:fn daily_prune_throttle_runs_at_most_once_per_day; 683:fn clock_going_backwards_does_not_mass_prune; 721:fn stored_body_len; 733:fn bytes_out_is_exact_utf8_len_for_json_and_lean_including_multibyte_and_escaping; 774:fn first_auto_read_filesystem_full_saved_bytes_is_zero; 794:fn second_auto_store_receipt_saved_bytes_equals_stored_body_byte_len; 819:fn store_plus_explicit_full_saved_bytes_is_zero; 842:fn explicit_receipt_first_read_saved_bytes_is_zero; 862:fn error_response_bytes_out_equals_rendered_error_bytes; 877:fn receipt_rate_denominator_not_polluted_by_explicit_full_or_receipt; 908:fn stale_tristate_true_false_and_null_not_evaluated_are_distinct; 927:fn unindexed_status_does_not_count_as_stale; 944:fn privacy_sentinels_from_context_recall_struct_unknown_flag_and_clap_error_are_absent_from_usage_db; 993:fn global_vs_unknown_project_distribution_are_separated; 1028:fn empty_db_yields_stable_zero_report; 1060:fn current_usage_call_absent_from_own_report_but_present_in_the_next; 1074:fn golden_json_and_lean_snapshots; 1195:fn recorded_ts_is_utc_unix_ms; 1222:fn a_report_names_the_machine_and_says_when_a_db_holds_two; 1289:fn the_machine_id_is_stable_across_a_deleted_database; 1314:fn the_machine_id_never_carries_a_hostname
- `evals/src/main.rs` (1251 LOC, rust)  — 30:fn print_report; 47:fn at; 55:fn has; 65:const RUN_AGENTS_FLAGS; 79:const VERIFY_IMPACT_FLAGS; 80:const RECALL_EXP_FLAGS; 81:const HOOK_PROBE_FLAGS; 82:const ADOPT_MINE_FLAGS; 91:const SUMMARIZE_FLAGS; 92:const DEMAND_FLAGS; 100:const USAGE_TOP; 102:const USAGE_RUN_AGENTS; 103:const USAGE_VERIFY_IMPACT; 105:const USAGE_SUMMARIZE; 106:const USAGE_DEMAND; 107:const USAGE_HOOK_PROBE; 109:const USAGE_ADOPT_MINE; 110:const USAGE_RECALL_EXP; 116:fn delay_secs; 123:fn split_only; 131:fn only_matches; 138:fn sort_rows; 152:fn run_status_json; 172:struct RunStatus; 178:impl impl RunStatus; 179:fn drop; 195:struct BatchRead; 201:impl impl BatchRead; 202:fn load; 217:fn field; 225:fn report; 247:fn problem; 291:fn wants_help; 295:fn check_flags; 316:fn guard_options; 324:fn sanitize; 339:fn venue_from; 352:fn venue_head; 369:fn run_cell; 402:fn run_agents; 616:fn verify_impact_main; 640:fn recall_exp_main; 660:fn hook_probe_main; 700:fn summarize_main; 746:fn demand_main; 850:fn adopt_mine_main; 932:fn main; 969:mod option_guard; 972:fn v; 977:fn a_help_flag_is_not_mistaken_for_a_run; 985:fn unknown_options_are_refused_with_the_subcommand_usage; 992:fn every_recognised_option_is_listed; 1007:fn equals_form_is_refused_rather_than_dropped; 1014:fn bare_flags_are_matched_exactly; 1021:fn positionals_stay_positionals; 1027:mod venue; 1030:fn v; 1035:fn a_missing_venue_is_refused_with_the_flag_in_the_message; 1043:fn a_venue_that_is_not_a_directory_names_the_path; 1057:fn an_existing_directory_is_accepted; 1066:mod sampling_window; 1069:fn v; 1074:fn only_selects_several_tasks_at_once; 1085:fn a_refused_typo_selects_nothing_rather_than_everything; 1091:fn delay_defaults_to_no_wait; 1098:fn delay_refuses_nonsense_instead_of_running_immediately; 1110:mod batch_accounting; 1113:fn row; 1118:fn a_partial_batch_says_so_instead_of_looking_complete; 1139:fn rows_sort_the_same_way_whichever_order_threads_finished_in; 1175:mod whitelist_coverage; 1182:fn every_option_the_parser_asks_for_is_whitelisted; 1220:fn the_scan_itself_finds_the_real_options; 1229:mod batch_consumption; 1233:fn status; 1237:fn batch; 1246:fn a_whole_batch_raises_no_flag_and_reports_its_counts; 1264:fn a_short_batch_is_named_with_exactly_how_many_cells_are_missing; 1274:fn a_killed_batch_is_caught_by_the_count_disagreement; 1284:fn a_sidecar_with_no_rows_at_all_is_still_a_lost_batch; 1290:fn pre_f18_artefacts_are_not_called_suspicious; 1302:fn an_incomplete_flag_without_a_count_gap_is_still_a_problem; 1311:fn load_reads_the_sidecar_beside_rows_json
- `install.sh` (1063 LOC, shell)
- `rust/tests/readings.rs` (1000 LOC, rust)  — 18:const BODY; 20:struct Harness; 27:fn setup; 55:fn note_count; 60:fn fts_count; 65:fn fts_match_database; 74:fn json_keys_in_order; 85:fn payload_json; 89:fn read_auto; 108:struct CountingFs; 114:impl impl CountingFs; 115:fn new; 122:fn open_reads; 127:impl impl CountingFs; 128:fn canonicalize; 131:fn metadata; 135:fn open_read; 141:struct FailFs; 148:impl impl FailFs; 149:fn raw; 157:fn kind; 165:fn matches; 168:fn clone_err; 176:impl impl FailFs; 177:fn canonicalize; 180:fn metadata; 186:fn open_read; 197:struct RaceFs; 202:impl impl RaceFs; 203:fn new; 211:impl impl RaceFs; 212:fn canonicalize; 215:fn metadata; 218:fn open_read; 239:fn restore_mtime_after_equal_length_edit; 257:fn reading_notes_require_an_indexed_project; 277:fn first_auto_is_filesystem_full_second_auto_is_store_receipt; 341:fn second_content_full_returns_body_byte_identical_to_first; 378:fn whole_file_note_serves_subrange_after_hashing_whole_source; 438:fn a_partial_note_never_masquerades_as_a_whole_file_cache_entry; 451:fn an_omitted_end_line_caches_the_requested_start_through_eof; 483:fn unchanged_reading_notes_survive_a_real_full_reindex; 499:fn fts_recall_returns_stored_readings_and_drops_them_after_hash_mismatch; 533:fn reading_rejects_paths_outside_the_indexed_project_and_invalid_ranges; 552:fn equal_length_edit_with_restored_mtime_must_not_serve_stale; 579:fn metadata_identical_and_hash_identical_still_hashes_before_store_hit; 608:fn subrange_from_whole_file_note_hashes_the_whole_source_not_the_subrange; 635:fn content_receipt_on_first_miss_persists_and_returns_filesystem_receipt; 678:fn invalid_content_mode_reports_provided_and_allowed; 701:fn classify_validation_failure_table; 745:fn recall_enoent_prunes_notes_and_fts_and_succeeds_empty; 758:fn recall_eio_keeps_notes_and_emits_exact_validation_error_fields; 807:fn recall_enotdir_retains; 831:fn recall_emfile_retryable_true_eacces_retryable_false; 870:fn recall_language_not_found_without_raw_enoent_retains; 890:fn recall_multi_candidate_eio_fail_closed_no_partial_results; 906:fn hash_mismatch_rebuild_uses_already_read_bytes_no_second_read; 941:fn pre_post_metadata_race_retries_once_then_validation_error; 991:fn recall_not_regular_file_retains; 1005:fn recall_trims_to_head_lines_unless_full_content; 1024:fn recall_rejects_invalid_limit_and_empty_query; 1038:fn read_count_does_not_increment_before_verification
- `rust/tests/context.rs` (952 LOC, rust)  — 25:static ENV_LOCK; 27:fn env_guard; 31:fn fake_ag; 36:fn with_vars; 64:const SAMPLE; 84:const OLD_RUST_YML; 87:fn make_project; 103:fn indexed; 121:fn git_init; 159:fn the_default_budget_is_1500_tokens; 167:fn an_exact_symbol_name_resolves_without_touching_fts; 189:fn a_non_symbol_query_falls_back_to_fts; 210:fn seeds_carry_depth_1_neighbours; 236:fn ambiguous_neighbours_are_dropped_unless_explicitly_requested; 286:fn an_unresolvable_reference_is_inlined_on_the_fly_and_never_persisted; 317:fn the_emitted_json_actually_fits_the_budget_and_reports_truncation; 358:fn an_unknown_query_returns_an_empty_packet_rather_than_throwing; 379:fn context_never_invokes_struct; 389:fn seed_content_is_truncated_by_default_and_restorable_with_full_content; 451:fn short_content_is_untouched_and_not_flagged; 472:fn a_rust_symbol_returns_only_its_function_body_not_the_rest_of_a_large_file; 509:fn canonical_owner_strips_per_segment_generics_and_normalizes_whitespace; 546:fn parse_scan; 571:fn rust_pack_rules_are_mutually_exclusive_and_capture_owner; 635:fn six_impls_all_named_run_store_qualified_names; 666:fn qualified_query_for_the_sixth_run_is_still_exact; 695:fn trait_default_method_body_via_content_full; 729:fn trait_impl_collision_keeps_both_type_run_in_stable_order; 772:fn type_method_json_and_lean_and_nonexistent_qualified_is_none_without_fts; 848:fn parse_symbol_query_splits_on_the_last_colon_colon_outside_generics; 878:fn method_record_without_owner_is_malformed_extraction; 942:fn hash_pack_with_rust_yml; 960:fn rust_yml_change_moves_the_pack_hash_and_old_index_requires_full_rebuild
- `rust/src/graph.rs` (906 LOC, rust)  — 10:struct ConfidenceScore; 16:const CONFIDENCE_SCORE; 23:struct RelationshipRow; 40:struct UnresolvedInline; 47:struct Neighbor; 60:struct Dependent; 70:struct ContainingChunk; 80:fn build_import_map; 90:fn posix_dirname; 102:fn posix_normalize; 140:fn posix_join; 155:fn strip_last_ext; 169:fn imported_path_prefixes; 195:fn internal_rust_path_target; 205:fn module_segments; 220:fn expand_use_path; 263:fn split_call_path; 289:struct Candidate; 298:fn load_candidates; 327:fn ends_with_segments; 340:fn resolve_candidates; 421:type ReceiverCandidate; 431:struct ReceiverIndex; 435:impl impl ReceiverIndex; 436:fn build; 456:fn candidates; 463:fn symbol_owner; 469:fn norm_name; 500:fn receiver_binds; 540:fn resolve_edge_targets; 565:fn resolve_targets; 624:fn relationship_rows_for_file; 642:fn relationship_rows_for_symbol_map; 663:fn relationship_rows_for_symbol_map_with_index; 738:fn unresolved_inline; 746:fn get_neighbors; 776:fn get_transitive_dependents; 809:fn containment_join; 838:const INSERT_REL; 844:fn insert_relationship; 867:fn rebuild_relationships
- `rust/tests/db.rs` (896 LOC, rust)  — 12:static ENV_LOCK; 14:fn env_guard; 18:fn with_var; 40:fn home_dir; 44:fn fresh; 52:fn project_id_is_a_stable_sha256_of_the_real_path; 66:fn db_path_lands_under_the_cortex_ng_cache_keyed_by_project_id; 81:fn ensure_schema_is_idempotent_and_records_the_schema_version; 97:fn ensure_schema_upgrades_a_v1_database_with_the_reading_notes_fts_layer; 122:fn schema_uses_the_v6_column_names_required_by_the_spec; 153:fn relationships_primary_key_is_the_composite_triple; 179:fn fts_triggers_mirror_chunk_writes; 215:fn zero_target_relationships_are_impossible_target_chunk_id_is_not_null; 233:fn list_projects_enumerates_every_indexed_project_in_the_cache_dir; 271:fn delete_project_removes_only_that_project_db_and_reports_what_it_did; 299:struct CodeErr; 304:impl impl CodeErr; 305:fn fmt; 310:impl impl CodeErr; 311:fn sqlite_code; 318:fn with_busy_retry_retries_sqlite_busy_and_gives_up_after_three_retries; 352:fn with_busy_retry_converts_a_full_or_corrupt_db_into_storage_full; 376:const V3_SHAPED_DB; 419:fn columns; 429:fn a_v3_database_is_upgraded_in_place_and_its_rows_survive_the_column_addition; 469:fn the_call_form_column_is_checked_on_upgraded_and_fresh_databases; 503:fn re_running_the_v4_upgrade_is_a_no_op; 518:fn a_missing_table_is_classified_as_an_outdated_schema_not_contention; 538:fn an_ordinary_sqlite_failure_is_still_storage_busy; 554:fn migrating_a_real_v3_database_to_v5_preserves_and_aligns_every_row; 621:fn a_stale_v5_temporary_table_does_not_wedge_the_next_upgrade; 645:fn mark_schema_only; 652:fn mark_indexed; 676:fn a_path_resolves_to_its_nearest_indexed_ancestor; 719:fn a_deleted_path_still_resolves_through_its_parents; 746:fn an_unreadable_database_stops_the_walk_rather_than_diverting_it; 779:fn list_projects_reports_the_schema_and_extractor_each_index_was_built_with; 826:fn a_metadata_read_that_fails_is_unreadable_rather_than_drifted; 865:fn a_cache_directory_that_will_not_enumerate_is_not_an_empty_one; 903:fn the_usage_recorder_is_not_a_project; 927:fn an_index_that_will_not_answer_is_reported_rather_than_skipped
- `rust/src/readings.rs` (867 LOC, rust)  — 17:const DEFAULT_RECALL_LIMIT; 18:const RECALL_HEAD_LINES; 20:const JS_MAX_SAFE_INTEGER; 23:enum ContentMode; 30:struct ReadReceipt; 41:struct ReadFull; 54:enum ReadPayload; 60:struct RecallReading; 71:struct RecallPayload; 79:struct ValidationErrorDetail; 90:enum NoteDisposition; 96:struct ClassifiedFailure; 104:struct FileMeta; 113:struct OpenReadError; 118:trait SourceFs; 124:struct RealFs; 126:impl impl RealFs; 127:fn canonicalize; 131:fn metadata; 135:fn open_read; 149:fn parse_content_mode; 165:fn fragment_hash_prefix; 170:fn classify_validation_failure; 186:fn read_fragment; 208:fn read_fragment_with_fs; 305:fn recall_readings; 316:fn recall_readings_with_fs; 402:fn classify_os_code; 434:fn sha256_hex; 438:fn file_meta; 461:fn now_ms; 468:fn cort_validation; 475:fn require_indexed; 492:fn require_positive_line; 502:struct ResolvedPath; 507:fn resolve_project_file; 544:fn inside_project; 555:struct NoteRow; 564:fn load_notes; 588:fn is_covering; 598:struct ValidatedSource; 604:enum StableRead; 611:enum FileCheck; 617:fn same_identity; 621:fn io_outcome; 637:fn retained; 649:fn read_stable; 683:fn prune_notes_for_file; 691:fn update_file_metadata; 700:fn commit_store_hit; 732:fn slice_stored; 741:fn slice_lines; 746:fn effective_mode; 760:fn build_read_payload; 795:fn persist_from_bytes; 875:struct RecallRow; 885:fn load_recall_candidates; 914:fn trim_content
- `rust/tests/graph.rs` (853 LOC, rust)  — 19:const SAMPLE; 41:fn is_indexable; 64:struct Indexed; 71:fn write_project; 84:fn insert_chunk; 106:fn index_files; 177:fn indexed; 183:fn confidence_constants_match_the_spec_exactly; 191:fn a_single_hit_call_resolves_to_one_inferred_row; 218:fn an_ambiguous_call_writes_one_row_per_target_with_score_0_5_over_n; 249:fn a_call_with_no_resolvable_target_writes_no_row_at_all; 277:fn unresolved_inline_is_the_on_the_fly_shape_and_carries_no_chunk_id; 287:fn a_symbol_never_calls_itself; 312:fn get_neighbors_returns_depth_1_edges_in_both_directions_capped; 332:fn get_transitive_dependents_walks_the_reverse_edge_up_to_depth; 357:fn build_import_map_keys_only_the_module_specifiers_of_import_edges; 380:fn resolve_targets_prefers_files_reachable_through_the_import_map; 410:const RECEIVER_FIXTURE; 430:fn receiver_edge; 440:fn rows_for; 453:fn chunk_ids_named; 467:fn a_receiver_call_attaches_when_its_method_name_belongs_to_exactly_one_symbol; 498:fn a_receiver_call_is_not_attached_when_two_symbols_answer_to_the_name; 522:fn a_receiver_call_into_std_attaches_nothing_and_is_invisible_to_the_gate; 543:fn the_gate_does_not_touch_the_recall_bare_names_already_had; 570:fn receiver_candidates_are_counted_by_the_last_segment_of_a_qualified_name; 613:fn a_relationship_keeps_the_earliest_call_site_when_one_function_calls_twice; 639:fn edges_are_walked_in_source_order_so_the_reported_line_does_not_depend_on_subprocess_output; 667:fn a_receiver_call_never_binds_to_an_ownerless_symbol; 676:fn self_calls_bind_to_the_enclosing_impl_and_to_nothing_else; 707:fn a_receiver_binds_when_its_name_is_the_owner_s_name_in_any_rust_shape; 731:fn a_receiver_that_does_not_look_like_the_owner_is_refused_even_when_the_name_is_unique; 759:fn module_segments_strip_src_and_mod_components; 777:fn expand_use_path_fans_out_brace_groups_and_drops_crate; 797:fn a_qualified_rust_call_resolves_through_the_module_path_suffix; 821:fn a_use_path_disambiguates_a_bare_rust_call_between_modules; 843:fn a_qualified_call_matching_no_module_stays_unresolved; 873:fn a_std_module_qualifier_that_matches_a_local_module_file_still_attaches
- `rust/tests/incremental.rs` (848 LOC, rust)  — 16:const SAMPLE; 36:fn make_project; 52:fn git; 68:fn git_project; 93:fn an_extractor_version_mismatch_forces_a_full_rebuild; 106:fn no_changes_means_nothing_is_reindexed; 115:fn an_edited_file_is_reindexed_and_its_chunks_replaced; 139:fn a_touched_but_identical_file_is_skipped_without_a_write; 167:fn a_new_untracked_file_is_picked_up_via_git_ls_files_others; 190:fn a_deleted_file_drops_its_chunks_fts_rows_and_file_state; 224:fn an_interrupt_keeps_already_committed_files_and_does_not_advance_git_head; 285:fn a_non_git_directory_degrades_to_a_full_index; 297:fn remove_file_and_reindex_one_file_each_run_in_their_own_transaction; 319:fn rel_count; 324:fn edge_exists; 337:fn an_incremental_reindex_of_a_callee_keeps_incoming_edges_from_unchanged_files; 378:fn an_incremental_reindex_reapplies_edges_from_files_that_only_grew_a_new_callee; 403:fn a_full_index_persists_the_raw_edges_needed_to_rebuild_the_graph; 427:fn a_pending_graph_is_reported_stale_even_when_every_file_hash_matches; 443:fn a_completed_incremental_index_clears_the_pending_graph_marker; 467:fn a_v3_index_is_upgraded_and_its_rebuilt_graph_carries_forms_and_call_sites; 610:fn a_bare_and_a_receiver_call_of_the_same_name_on_one_line_are_two_rows; 669:fn a_head_that_moved_without_dirtying_the_tree_is_reindexed_not_just_restamped; 715:fn an_untracked_file_that_is_deleted_leaves_the_index; 754:fn chunk_rows; 771:const IGNORED_SAMPLE; 783:fn an_indexed_file_git_will_not_speak_for_is_reexamined_when_it_changes; 873:fn an_indexed_file_the_walk_no_longer_covers_is_removed
- `rust/tests/coverage.rs` (837 LOC, rust)  — 11:fn indexed; 38:fn coverage_of; 55:fn a_unique_receiver_call_becomes_an_edge_with_a_line_to_check; 89:fn an_ambiguous_receiver_call_attaches_nothing_and_still_shows_up_as_a_gap; 131:fn a_call_the_pack_did_extract_is_not_counted_as_a_gap; 156:fn a_name_inside_a_string_is_attributed_to_quoted_and_sorted_last; 181:fn a_file_that_never_entered_the_index_is_a_blind_spot_and_says_so; 205:fn absence_of_a_signal_is_never_named_like_proof; 254:fn a_generated_bundle_is_not_allowed_to_look_like_a_missed_caller; 306:fn a_blind_file_is_never_a_clean_bill_of_health; 357:fn a_file_with_no_chunks_is_advisory_and_does_not_flip_every_seed; 408:fn a_single_quoted_string_is_not_reported_as_a_bare_mention; 442:fn two_mentions_on_one_line_collapse_into_one_row_that_says_so; 470:fn a_symbol_that_is_not_indexed_reports_itself_instead_of_failing; 493:fn a_file_too_big_to_read_is_reported_as_skipped_instead_of_clean; 529:fn qualification_and_word_boundaries_are_parsed_the_way_the_caller_reads_them; 554:fn a_declaration_line_is_never_a_call_even_when_the_declaration_is_not_a_chunk; 584:fn a_real_gap_in_a_partially_covered_file_outranks_comment_noise_from_an_uncovered_one; 624:fn an_import_the_extractor_saw_but_could_not_resolve_is_a_pack_attested_drop; 656:fn an_import_whose_file_already_reaches_the_seed_is_suppressed; 677:fn a_brace_import_reports_each_name_the_extractor_could_not_resolve; 716:fn gap_count_is_the_number_the_boolean_reads_not_the_mention_layers_alone; 766:fn a_gitignored_file_is_disclosed_as_unindexed_rather_than_silently_dropped; 819:fn a_resolved_type_reference_is_not_also_reported_as_a_mention_gap; 845:fn a_dropped_type_reference_is_reported_as_an_unresolved_extraction
- `tests/install-smoke.sh` (826 LOC, shell)
- `rust/tests/settings_toml.rs` (806 LOC, rust)  — 14:fn tmp; 23:fn read; 30:fn cmd_at; 40:fn group_hook_count; 51:fn group_count; 60:fn with_existing_hooks; 82:fn installs_into_a_file_that_does_not_exist_yet; 100:fn a_second_install_changes_nothing_and_does_not_rewrite; 112:fn every_hook_the_user_already_had_survives; 132:fn a_moved_binary_updates_the_entry_instead_of_adding_a_second_one; 157:fn remove_takes_ours_out_and_leaves_theirs; 173:fn uninstalling_a_hook_we_never_installed_writes_nothing; 183:fn remove_leaves_no_empty_scaffolding_in_a_file_that_had_no_hooks; 197:fn a_config_file_we_cannot_parse_is_refused_not_overwritten; 209:fn check_can_report_the_wired_command_without_touching_the_file; 223:fn with_hand_wired_duplicates; 247:fn a_command_with_a_redirection_suffix_is_still_ours; 257:fn a_redeploy_collapses_hand_wired_duplicates_to_one_entry; 274:fn the_surviving_entry_keeps_no_hand_typed_field; 294:fn collapsing_duplicates_is_idempotent; 314:fn remove_takes_out_a_hand_wired_entry_too; 328:fn a_malformed_group_does_not_hide_the_entry_behind_it; 352:fn a_command_merely_mentioning_the_word_is_not_ours; 375:fn a_third_party_binary_named_hook_suggest_is_not_ours; 419:fn a_hooks_key_of_the_wrong_type_is_refused_not_replaced; 429:fn a_pretooluse_of_the_wrong_type_is_refused_not_replaced; 439:fn the_users_own_empty_group_survives_a_collapse; 475:fn the_users_own_empty_group_survives_a_remove; 514:fn trust_block; 520:fn append; 527:fn a_wired_entry_with_no_state_table_reads_as_untrusted; 536:fn a_trusted_hash_at_our_position_reads_as_trusted; 548:fn the_path_half_of_the_key_is_not_compared; 558:fn a_trusted_hash_at_a_different_position_does_not_count; 569:fn a_state_entry_without_a_hash_does_not_count; 584:fn install_preserves_a_trust_table_it_did_not_write; 615:fn installed_command_still_agrees_with_installed_entry; 633:fn kimi_tmp; 642:fn kimi_entries; 671:fn installing_the_refresh_hook_does_not_disturb_the_suggest_hook; 698:fn a_moved_binary_updates_each_event_in_place; 722:fn remove_takes_both_events_and_nothing_else; 746:fn the_subcommand_is_what_identifies_which_of_ours_an_entry_is; 768:fn an_install_and_remove_cycle_returns_the_file_exactly_as_it_was; 812:fn a_redeploy_repairs_a_stale_matcher_even_when_the_command_is_unchanged; 849:fn a_shared_groups_matcher_is_left_alone

*Generated at 2026-09-06T12:44:43Z by claudecat* — facts from manifests + AST; framework/entry may be inferred from deps/paths where manifest lacks them.
<!-- claudecat:auto:end -->
<!-- claudecat:guardrails:begin -->
<!-- 技術決策 / Guardrails：每行一條，例如 `2D tilemap + Macroquad（禁 Python/3D）`、`插件一律裝在 Claude Code 內`。claudecat 只在此區不存在時建立，之後永不覆寫。 -->
<!-- claudecat:guardrails:end -->
