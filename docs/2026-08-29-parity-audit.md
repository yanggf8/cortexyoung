# cort Rust port parity audit (Job E1, 2026-08-29)

Read-only comparison of frozen JS `tests/*.test.js` against `rust/tests/*.rs`, keyed to `docs/superpowers/plans/2026-08-28-rust-port-spec.md` §6 (176 rows) and superseded where they conflict by `docs/superpowers/plans/2026-08-28-codex-fix-proposal.md` §§1–4. Assertions compared from test bodies. `npm test` / `cargo test` were not run.

Verdicts:

- **PASS** — a Rust test exists and asserts at least as strongly as the JS test for behavior the proposal did not change.
- **SUPERSEDED-by-proposal** — JS asserted the old contract; Rust tests the proposal instead.
- **DIVERGES** — a Rust test exists but is weaker than JS, or differs from both JS and the proposal in an unintended way.
- **MISSING** — no Rust test covers the row.

Covered = PASS + SUPERSEDED-by-proposal.

---

## Summary

| job | rows | PASS | SUPERSEDED-by-proposal | DIVERGES | MISSING |
|---|---:|---:|---:|---:|---:|
| B (ast-grep / budget / db / fts) | 29 | 29 | 0 | 0 | 0 |
| C1 (pack / chunker / graph) | 29 | 28 | 1 | 0 | 0 |
| C2 (indexer / incremental / staleness) | 22 | 22 | 0 | 0 | 0 |
| C3 (readings) | 8 | 5 | 3 | 0 | 0 |
| D cort (context / struct / impact / render / cli) | 47 | 45 | 2 | 0 | 0 |
| D eval (`evals/*.mjs`, out of crate) | 41 | 0 | 0 | 0 | 41 |
| **total** | **176** | **129** | **6** | **0** | **41** |

**Covered: 135 / 176** (cort-body 135 / 135 after the C3-6 fix; D-48..D-88 are spec-skip eval rows).

### SUPERSEDED-by-proposal (intended)

| id | proposal | JS asserted | Rust test |
|---|---|---|---|
| C1-10 | §4 `Type::method` | `symbol_name === 'work'` | `chunker.rs::rust_functions_and_impl_methods_are_symbol_scoped_ast_chunks` looks up `Worker::work` |
| C3-2 | §1 receipt | second read `content === first.content` | `readings.rs::first_auto_is_filesystem_full_second_auto_is_store_receipt` (store/receipt, no `content` field) |
| C3-3 | §1 + §2 | whole-file note serves subrange “without another filesystem payload” | `readings.rs::whole_file_note_serves_subrange_after_hashing_whole_source` (`open_reads==1`, then receipt) |
| C3-5 | §1 receipt | second omit-end read has `content` | `readings.rs::an_omitted_end_line_caches_the_requested_start_through_eof` (store/receipt; body via `--content full`) |
| D-41 | §1 lean receipt | lean store read includes `fn work()` | `render.rs::lean_reading_output_identifies_cache_provenance_and_keeps_stored_content` (receipt has no body) |
| D-47 | §1 CLI receipt | second CLI read `source=store` (JS does not check `content`) | `cli.rs::read_persists_a_fragment_and_recall_finds_it_through_fts` also locks `content_mode=receipt` and absent `content` |

### MISSING or unintended DIVERGE

| id | verdict | explanation |
|---|---|---|
| C3-6 | PASS (fixed 2026-08-29 post-audit) | Original Rust test only simulated the reindex by DELETE-ing chunks/file_state; coordinator replaced it with `unchanged_reading_notes_survive_a_real_full_reindex` which calls the real `full_index` (resolves ast-grep, walks, rebuilds) and asserts notes survive + `read_count=2`. |
| D-48..D-88 | MISSING | Eval harness (`evals/*.mjs`). Spec §6: not in the rust crate; listed so 176 ids stay complete. |

No other MISSING cort-body rows. No rust test still asserts JS-era store-hit full body, `stat_matches` skip-hash, or bare Rust method names on C1-10.

Proposal TDD extras (not §6 rows): `rust/tests/readings.rs` covers §1–§3 library tests; `rust/tests/context.rs` covers §4 TDD-1..8. Two proposal TDD items have **no** rust test at the named layer: §1 TDD.5 (CLI `--content` matrix / missing value) and §2 TDD.2 (recall after equal-length edit + restored mtime). Those are not §6 row ids.

---

## B — foundation (29)

| id | behavior | JS source | Rust test | verdict | notes |
|---|---|---|---|---|---|
| B-1 | real ast-grep matches pin | `tests/ast-grep.test.js` / `resolves the real ast-grep and it matches the pin` | `rust/tests/ast_grep.rs::resolves_the_real_ast_grep_and_it_matches_the_pin` | PASS | |
| B-2 | missing binary fail-closed | `tests/ast-grep.test.js` / `missing binary is fail-closed` | `rust/tests/ast_grep.rs::missing_binary_is_fail_closed` | PASS | |
| B-3 | wrong version found/expected + `toJSON` `{error,detail}` | `tests/ast-grep.test.js` / `wrong version is fail-closed with found/expected detail` | `rust/tests/ast_grep.rs::wrong_version_is_fail_closed_with_found_expected_detail` | PASS | |
| B-4 | hung subprocess → `ast_grep_timeout` | `tests/ast-grep.test.js` / `a hung subprocess raises ast_grep_timeout` | `rust/tests/ast_grep.rs::a_hung_subprocess_raises_ast_grep_timeout` | PASS | `timeoutMs`/`timeout_ms: 150` |
| B-5 | exec returns code/stdout/stderr separately | `tests/ast-grep.test.js` / `execAstGrep returns code, stdout and stderr separately` | `rust/tests/ast_grep.rs::exec_ast_grep_returns_code_stdout_and_stderr_separately` | PASS | `code=1`, `OUT\n` / `ERR\n` |
| B-6 | 4 chars/token, round up | `tests/budget.test.js` / `token estimate is four characters per token, rounded up` | `rust/tests/budget.rs::token_estimate_is_four_characters_per_token_rounded_up` | PASS | `""→0`, `"abcd"→1`, `"abcde"→2` |
| B-7 | applyBudget keeps while cumulative fits | `tests/budget.test.js` / `applyBudget keeps items while the cumulative rendered size fits` | `rust/tests/budget.rs::apply_budget_keeps_items_while_the_cumulative_rendered_size_fits` | PASS | kept 2, truncated true |
| B-8 | no truncation when everything fits | `tests/budget.test.js` / `applyBudget reports no truncation when everything fits` | `rust/tests/budget.rs::apply_budget_reports_no_truncation_when_everything_fits` | PASS | |
| B-9 | always keep at least one item | `tests/budget.test.js` / `applyBudget always keeps at least one item so the answer is never empty` | `rust/tests/budget.rs::apply_budget_always_keeps_at_least_one_item_so_the_answer_is_never_empty` | PASS | |
| B-10 | project id is stable sha256 of path | `tests/db.test.js` / `project id is a stable sha256 of the real path` | `rust/tests/db.rs::project_id_is_a_stable_sha256_of_the_real_path` | PASS | 64 lowercase hex |
| B-11 | db path under cortex-ng cache | `tests/db.test.js` / `db path lands under the cortex-ng cache keyed by project id` | `rust/tests/db.rs::db_path_lands_under_the_cortex_ng_cache_keyed_by_project_id` | PASS | |
| B-12 | ensureSchema idempotent + SCHEMA_VERSION | `tests/db.test.js` / `ensureSchema is idempotent and records the schema version` | `rust/tests/db.rs::ensure_schema_is_idempotent_and_records_the_schema_version` | PASS | |
| B-13 | v1 upgrade adds reading_notes FTS | `tests/db.test.js` / `ensureSchema upgrades a v1 database with the reading-notes FTS layer` | `rust/tests/db.rs::ensure_schema_upgrades_a_v1_database_with_the_reading_notes_fts_layer` | PASS | |
| B-14 | V6 column names | `tests/db.test.js` / `schema uses the V6 column names required by the spec` | `rust/tests/db.rs::schema_uses_the_v6_column_names_required_by_the_spec` | PASS | |
| B-15 | relationships PK is the triple | `tests/db.test.js` / `relationships primary key is the composite triple` | `rust/tests/db.rs::relationships_primary_key_is_the_composite_triple` | PASS | |
| B-16 | FTS triggers mirror chunk writes | `tests/db.test.js` / `fts triggers mirror chunk writes` | `rust/tests/db.rs::fts_triggers_mirror_chunk_writes` | PASS | MATCH `'alpha'` 1 then 0 |
| B-17 | `target_chunk_id` NOT NULL | `tests/db.test.js` / `zero-target relationships are impossible: target_chunk_id is NOT NULL` | `rust/tests/db.rs::zero_target_relationships_are_impossible_target_chunk_id_is_not_null` | PASS | |
| B-18 | listProjects enumerates cache | `tests/db.test.js` / `listProjects enumerates every indexed project in the cache dir` | `rust/tests/db.rs::list_projects_enumerates_every_indexed_project_in_the_cache_dir` | PASS | |
| B-19 | deleteProject removes only that db | `tests/db.test.js` / `deleteProject removes only that project db and reports what it did` | `rust/tests/db.rs::delete_project_removes_only_that_project_db_and_reports_what_it_did` | PASS | |
| B-20 | SQLITE_BUSY 3 retries then give up | `tests/db.test.js` / `withBusyRetry retries SQLITE_BUSY and gives up after three retries` | `rust/tests/db.rs::with_busy_retry_retries_sqlite_busy_and_gives_up_after_three_retries` | PASS | success on 3rd; always-busy `always==4` → `storage_busy` |
| B-21 | SQLITE_FULL → `storage_full` | `tests/db.test.js` / `withBusyRetry converts a full or corrupt db into storage_full` | `rust/tests/db.rs::with_busy_retry_converts_a_full_or_corrupt_db_into_storage_full` | PASS | both sides only throw `SQLITE_FULL`; neither asserts `SQLITE_CORRUPT` |
| B-22 | FTS terms quoted | `tests/fts.test.js` / `each term is quoted so FTS operators cannot leak through` | `rust/tests/fts.rs::each_term_is_quoted_so_fts_operators_cannot_leak_through` | PASS | |
| B-23 | MAX_OR_TERMS truncates | `tests/fts.test.js` / `more than MAX_OR_TERMS terms truncates and reports it` | `rust/tests/fts.rs::more_than_max_or_terms_terms_truncates_and_reports_it` | PASS | |
| B-24 | empty query → `empty_query` | `tests/fts.test.js` / `an empty query is rejected loudly` | `rust/tests/fts.rs::an_empty_query_is_rejected_loudly` | PASS | |
| B-25 | keywordSearch finds symbol by name | `tests/fts.test.js` / `keywordSearch finds a symbol by name` | `rust/tests/fts.rs::keyword_search_finds_a_symbol_by_name` | PASS | JS `fullIndex(SAMPLE)`; Rust seeds SAMPLE-shaped rows. Result asserts match. |
| B-26 | punctuation does not blow MATCH | `tests/fts.test.js` / `keywordSearch survives punctuation that would otherwise be FTS syntax` | `rust/tests/fts.rs::keyword_search_survives_punctuation_that_would_otherwise_be_fts_syntax` | PASS | |
| B-27 | unicode61 CJK identifiers | `tests/fts.test.js` / `unicode61 tokenizing lets CJK identifiers through` | `rust/tests/fts.rs::unicode61_tokenizing_lets_cjk_identifiers_through` | PASS | `查詢使用者` → non-empty |
| B-28 | results scoped to project | `tests/fts.test.js` / `results are scoped to the project` | `rust/tests/fts.rs::results_are_scoped_to_the_project` | PASS | |
| B-29 | limit honoured | `tests/fts.test.js` / `the limit is honoured` | `rust/tests/fts.rs::the_limit_is_honoured` | PASS | `limit=1` → `len<=1` |

Extra (not B rows): `ast_grep.rs::fake_ast_grep_modes_empty_emit_and_preflight`; `errors.rs::{to_json_is_error_and_detail_not_code,display_matches_js_super_message,default_detail_is_json_null}`.

---

## C1 — pack / chunker / graph (29)

| id | behavior | JS source | Rust test | verdict | notes |
|---|---|---|---|---|---|
| C1-1 | pack files sorted + hash deterministic | `tests/pack.test.js` / `pack files are enumerated in sorted order and hash deterministically` | `rust/tests/pack.rs::pack_files_are_enumerated_in_sorted_order_and_hash_deterministically` | PASS | `len>=5`, absolute, 64 lowercase hex |
| C1-2 | extractor_version moves when a pack file changes | `tests/pack.test.js` / `extractor_version changes when any pack file changes` | `rust/tests/pack.rs::extractor_version_changes_when_any_pack_file_changes` | PASS | probe `typescript.yml` |
| C1-3 | TS tags + quoted `$SRC` | `tests/pack.test.js` / `the pack extracts chunks and edges from TypeScript with the expected tags` | `rust/tests/pack.rs::the_pack_extracts_chunks_and_edges_from_typescript_with_the_expected_tags` | PASS | tags exactly `chunk:class,chunk:function,chunk:method,edge:calls,edge:calls,edge:imports`; `$SRC="'./helper'"` |
| C1-4 | Python chunks/edges | `tests/pack.test.js` / `the pack extracts chunks and edges from Python` | `rust/tests/pack.rs::the_pack_extracts_chunks_and_edges_from_python` | PASS | imports `helper`,`os`; class 1; function 2 |
| C1-5 | PACK_DIR + sgconfig.yml exist | `tests/pack.test.js` / `PACK_DIR points at a real directory containing sgconfig.yml` | `rust/tests/pack.rs::pack_dir_points_at_a_real_directory_containing_sgconfig_yml` | PASS | |
| C1-6 | malformed scan lines skipped+counted | `tests/chunker.test.js` / `malformed lines are skipped and counted, valid ones survive` | `rust/tests/chunker.rs::malformed_lines_are_skipped_and_counted_valid_ones_survive` | PASS | total 3, malformed 1 |
| C1-7 | edge strings tab-separated | `tests/chunker.test.js` / `edge strings use the tab-separated pre-resolution form` | `rust/tests/chunker.rs::edge_strings_use_the_tab_separated_pre_resolution_form` | PASS | |
| C1-8 | file_content_hash covers chunks+edges | `tests/chunker.test.js` / `file_content_hash covers both chunk contents and edge strings` | `rust/tests/chunker.rs::file_content_hash_covers_both_chunk_contents_and_edge_strings` | PASS | |
| C1-9 | 1-indexed lines + V6 chunk ids | `tests/chunker.test.js` / `extractFile produces 1-indexed lines and V6-shaped chunk ids` | `rust/tests/chunker.rs::extract_file_produces_1_indexed_lines_and_v6_shaped_chunk_ids` | PASS | `alpha` line 2, `p:k.ts:2` |
| C1-10 | Rust fn/impl are symbol-scoped AST chunks | `tests/chunker.test.js` / `Rust functions and impl methods are symbol-scoped AST chunks` | `rust/tests/chunker.rs::rust_functions_and_impl_methods_are_symbol_scoped_ast_chunks` | SUPERSEDED-by-proposal | JS `symbol_name === 'work'`; Rust `Worker::work` (proposal §4). Line/content/`chunk_source=ast` still asserted. |
| C1-11 | innermost containing chunk owns edges | `tests/chunker.test.js` / `edges are attributed to the innermost containing chunk` | `rust/tests/chunker.rs::edges_are_attributed_to_the_innermost_containing_chunk` | PASS | quotes stripped on import `./helper` |
| C1-12 | unparsable file → one unparsed FTS chunk | `tests/chunker.test.js` / `a file ast-grep cannot parse becomes a single unparsed FTS-only chunk` | `rust/tests/chunker.rs::a_file_ast_grep_cannot_parse_becomes_a_single_unparsed_fts_only_chunk` | PASS | |
| C1-13 | all-malformed stream → unparsed, no throw | `tests/chunker.test.js` / `an all-malformed scan stream degrades that file to unparsed and never throws` | `rust/tests/chunker.rs::an_all_malformed_scan_stream_degrades_that_file_to_unparsed_and_never_throws` | PASS | |
| C1-14 | 90%+ malformed still indexes survivor | `tests/chunker.test.js` / `a 90%-malformed scan stream still indexes the surviving record — scan never aborts` | `rust/tests/chunker.rs::a_90_percent_malformed_scan_stream_still_indexes_the_surviving_record_scan_never_aborts` | PASS | unparsed=false, malformed=19, `ok` |
| C1-15 | timeout → unparsed, not abort | `tests/chunker.test.js` / `a scan that times out degrades that file to unparsed instead of aborting` | `rust/tests/chunker.rs::a_scan_that_times_out_degrades_that_file_to_unparsed_instead_of_aborting` | PASS | hang + 200ms |
| C1-16 | spawn failure still propagates | `tests/chunker.test.js` / `a spawn failure still propagates — only timeout degrades to unparsed` | `rust/tests/chunker.rs::a_spawn_failure_still_propagates_only_timeout_degrades_to_unparsed` | PASS | `ast_grep_spawn_failed` |
| C1-17 | const-bound arrow/fn become function chunks | `tests/chunker.test.js` / `const-bound arrow and function expressions become function chunks` | `rust/tests/chunker.rs::const_bound_arrow_and_function_expressions_become_function_chunks` | PASS | `alpha`,`beta`,`handler` |
| C1-18 | map/alias do not become chunks | `tests/chunker.test.js` / `collection transforms and bare aliases do not become chunks` | `rust/tests/chunker.rs::collection_transforms_and_bare_aliases_do_not_become_chunks` | PASS | no `rows`/`gamma` |
| C1-19 | handler body calls attributed to handler | `tests/chunker.test.js` / `calls inside a const-bound handler get the handler as their source symbol` | `rust/tests/chunker.rs::calls_inside_a_const_bound_handler_get_the_handler_as_their_source_symbol` | PASS | |
| C1-20 | confidence constants 1.0 / 0.7 / 0.5 | `tests/graph.test.js` / `confidence constants match the spec exactly` | `rust/tests/graph.rs::confidence_constants_match_the_spec_exactly` | PASS | |
| C1-21 | single-hit call → one INFERRED 0.7 | `tests/graph.test.js` / `a single-hit call resolves to one INFERRED row` | `rust/tests/graph.rs::a_single_hit_call_resolves_to_one_inferred_row` | PASS | `alpha→helper` |
| C1-22 | ambiguous call 0.5/N | `tests/graph.test.js` / `an ambiguous call writes one row per target with score 0.5/N` | `rust/tests/graph.rs::an_ambiguous_call_writes_one_row_per_target_with_score_0_5_over_n` | PASS | 2 rows, 0.25 |
| C1-23 | no resolvable target → no row | `tests/graph.test.js` / `a call with no resolvable target writes no row at all` | `rust/tests/graph.rs::a_call_with_no_resolvable_target_writes_no_row_at_all` | PASS | no `unresolved_refs` table |
| C1-24 | unresolvedInline shape, no chunk id | `tests/graph.test.js` / `unresolvedInline is the on-the-fly shape and carries no chunk id` | `rust/tests/graph.rs::unresolved_inline_is_the_on_the_fly_shape_and_carries_no_chunk_id` | PASS | JS `'target_chunk_id' in u`; Rust type has no such field |
| C1-25 | symbol never calls itself | `tests/graph.test.js` / `a symbol never calls itself` | `rust/tests/graph.rs::a_symbol_never_calls_itself` | PASS | |
| C1-26 | getNeighbors depth-1, capped | `tests/graph.test.js` / `getNeighbors returns depth-1 edges in both directions, capped` | `rust/tests/graph.rs::get_neighbors_returns_depth_1_edges_in_both_directions_capped` | PASS | both sides only assert incoming `alpha` |
| C1-27 | transitive dependents to depth | `tests/graph.test.js` / `getTransitiveDependents walks the reverse edge up to depth` | `rust/tests/graph.rs::get_transitive_dependents_walks_the_reverse_edge_up_to_depth` | PASS | `['alpha','go']` |
| C1-28 | import map keys only import specifiers | `tests/graph.test.js` / `buildImportMap keys only the module specifiers of import edges` | `rust/tests/graph.rs::build_import_map_keys_only_the_module_specifiers_of_import_edges` | PASS | |
| C1-29 | resolveTargets prefers import map | `tests/graph.test.js` / `resolveTargets prefers files reachable through the import map` | `rust/tests/graph.rs::resolve_targets_prefers_files_reachable_through_the_import_map` | PASS | id contains `src/helper.ts` |

---

## C2 — indexer / incremental / staleness (22)

| id | behavior | JS source | Rust test | verdict | notes |
|---|---|---|---|---|---|
| C2-1 | walkFiles skips ignored + non-source | `tests/indexer.test.js` / `walkFiles skips ignored dirs and non-source extensions` | `rust/tests/indexer.rs::walk_files_skips_ignored_dirs_and_non_source_extensions` | PASS | `['src/alpha.ts','src/helper.ts']` |
| C2-2 | walk `.rs`; fullIndex stores free-fn fragments | `tests/indexer.test.js` / `walkFiles includes Rust sources and fullIndex stores function fragments` | `rust/tests/indexer.rs::walk_files_includes_rust_sources_and_full_index_stores_function_fragments` | PASS | free fns `small`,`other` — proposal §4 does not rename these |
| C2-3 | full index writes chunks, FTS, file_state, meta | `tests/indexer.test.js` / `a full index writes chunks, fts rows, file_state and meta` | `rust/tests/indexer.rs::a_full_index_writes_chunks_fts_rows_file_state_and_meta` | PASS | TS `go` remains a bare method name |
| C2-4 | reindex idempotent | `tests/indexer.test.js` / `re-indexing is idempotent — no duplicate chunks, no orphan fts rows` | `rust/tests/indexer.rs::re_indexing_is_idempotent_no_duplicate_chunks_no_orphan_fts_rows` | PASS | |
| C2-5 | unparsable file indexed as unparsed | `tests/indexer.test.js` / `an unparsable file is indexed as unparsed without failing the run` | `rust/tests/indexer.rs::an_unparsable_file_is_indexed_as_unparsed_without_failing_the_run` | PASS | |
| C2-6 | mid-run failure rolls back entire full index | `tests/indexer.test.js` / `the whole index is one transaction — a mid-run failure leaves the db untouched` | `rust/tests/indexer.rs::the_whole_index_is_one_transaction_a_mid_run_failure_leaves_the_db_untouched` | PASS | JS monkey-patches 2nd INSERT; Rust TEMP TRIGGER `RAISE(ABORT,'boom')`. Same end-state. |
| C2-7 | statusOf without touching ast-grep | `tests/indexer.test.js` / `statusOf reports the indexed project without touching ast-grep` | `rust/tests/indexer.rs::status_of_reports_the_indexed_project_without_touching_ast_grep` | PASS | neither side spies ast-grep; both assert fields only |
| C2-8 | extractor_version mismatch → full rebuild | `tests/incremental.test.js` / `an extractor_version mismatch forces a full rebuild` | `rust/tests/incremental.rs::an_extractor_version_mismatch_forces_a_full_rebuild` | PASS | generic `'stale-version-hash'`; §4 pack-hash TDD-8 lives in `context.rs`, does not replace this row |
| C2-9 | no changes → 0 reindexed | `tests/incremental.test.js` / `no changes means nothing is reindexed` | `rust/tests/incremental.rs::no_changes_means_nothing_is_reindexed` | PASS | |
| C2-10 | edited file reindexed, chunks replaced | `tests/incremental.test.js` / `an edited file is reindexed and its chunks replaced` | `rust/tests/incremental.rs::an_edited_file_is_reindexed_and_its_chunks_replaced` | PASS | `['helper','extra']` |
| C2-11 | touched-but-identical skipped without write | `tests/incremental.test.js` / `a touched-but-identical file is skipped without a write` | `rust/tests/incremental.rs::a_touched_but_identical_file_is_skipped_without_a_write` | PASS | both only check `updated_at` **if** `files_skipped==1` |
| C2-12 | untracked file via git ls-files --others | `tests/incremental.test.js` / `a new untracked file is picked up via git ls-files --others` | `rust/tests/incremental.rs::a_new_untracked_file_is_picked_up_via_git_ls_files_others` | PASS | |
| C2-13 | deleted file drops chunks, FTS, file_state | `tests/incremental.test.js` / `a deleted file drops its chunks, fts rows and file_state` | `rust/tests/incremental.rs::a_deleted_file_drops_its_chunks_fts_rows_and_file_state` | PASS | |
| C2-14 | interrupt keeps committed files; git_head not advanced | `tests/incremental.test.js` / `an interrupt keeps already-committed files and does NOT advance git_head` | `rust/tests/incremental.rs::an_interrupt_keeps_already_committed_files_and_does_not_advance_git_head` | PASS | Rust TRIGGER on `src/two.ts`; `done==1`, head unchanged |
| C2-15 | non-git directory → full | `tests/incremental.test.js` / `a non-git directory degrades to a full index` | `rust/tests/incremental.rs::a_non_git_directory_degrades_to_a_full_index` | PASS | |
| C2-16 | removeFile / reindexOneFile own transactions | `tests/incremental.test.js` / `removeFile and reindexOneFile each run in their own transaction` | `rust/tests/incremental.rs::remove_file_and_reindex_one_file_each_run_in_their_own_transaction` | PASS | both only assert skip + delete; title overclaims isolation |
| C2-17 | fresh clean tree not stale | `tests/staleness.test.js` / `a freshly indexed clean tree is not stale` | `rust/tests/staleness.rs::a_freshly_indexed_clean_tree_is_not_stale` | PASS | |
| C2-18 | dirty-but-identical (trailing comment) not stale | `tests/staleness.test.js` / `a dirty-but-semantically-identical file is NOT stale` | `rust/tests/staleness.rs::a_dirty_but_semantically_identical_file_is_not_stale` | PASS | |
| C2-19 | changed chunk body → stale | `tests/staleness.test.js` / `a changed chunk body makes the index stale` | `rust/tests/staleness.rs::a_changed_chunk_body_makes_the_index_stale` | PASS | |
| C2-20 | edge-only change → stale | `tests/staleness.test.js` / `an edge-only change makes the index stale` | `rust/tests/staleness.rs::an_edge_only_change_makes_the_index_stale` | PASS | |
| C2-21 | deleted file stale + reported | `tests/staleness.test.js` / `a deleted file makes the index stale and is reported` | `rust/tests/staleness.rs::a_deleted_file_makes_the_index_stale_and_is_reported` | PASS | |
| C2-22 | staleness uses projects.path, not cwd | `tests/staleness.test.js` / `staleness is computed against projects.path, not the cwd` | `rust/tests/staleness.rs::staleness_is_computed_against_projects_path_not_the_cwd` | PASS | |

Extra (not C2 rows): `indexer.rs::canonicalize_then_hash_equals_js_project_id_for_the_same_directory`.

---

## C3 — readings (8)

Proposal §§1–3 supersede JS on conflict.

| id | behavior | JS source | Rust test | verdict | notes |
|---|---|---|---|---|---|
| C3-1 | notes require indexed project | `tests/readings.test.js` / `reading notes require an indexed project` | `rust/tests/readings.rs::reading_notes_require_an_indexed_project` | PASS | `project_not_indexed` |
| C3-2 | first persist; unchanged repeat from store | `tests/readings.test.js` / `a first fragment read is persisted and an unchanged repeat comes from the store` | `rust/tests/readings.rs::first_auto_is_filesystem_full_second_auto_is_store_receipt` | SUPERSEDED-by-proposal | JS `second.content === first.content`. Rust second auto is store/receipt with no `content` field; `second_content_full_returns_body_byte_identical_to_first` covers `--content full`. Intended §1. |
| C3-3 | whole-file note serves later subranges | `tests/readings.test.js` / `a stored whole-file reading serves later subranges without another filesystem payload` | `rust/tests/readings.rs::whole_file_note_serves_subrange_after_hashing_whole_source` | SUPERSEDED-by-proposal | JS “without another filesystem payload”. Rust `CountingFs.open_reads()==1` then store/receipt; subrange hash prefix ≠ whole-file prefix. Intended §1+§2. |
| C3-4 | partial note is not a whole-file cache | `tests/readings.test.js` / `a partial note never masquerades as a whole-file cache entry` | `rust/tests/readings.rs::a_partial_note_never_masquerades_as_a_whole_file_cache_entry` | PASS | filesystem/full BODY |
| C3-5 | omitted end caches start through EOF | `tests/readings.test.js` / `an omitted end line caches the requested start through EOF` | `rust/tests/readings.rs::an_omitted_end_line_caches_the_requested_start_through_eof` | SUPERSEDED-by-proposal | JS second `source=store` **and** `content.startsWith('third line')`. Rust second is receipt; body via `--content full`. Intended §1. |
| C3-6 | notes survive a full re-index | `tests/readings.test.js` / `unchanged reading notes survive a full re-index` | `rust/tests/readings.rs::unchanged_reading_notes_survive_a_simulated_full_reindex` | DIVERGES | JS calls `fullIndex`. Rust `DELETE FROM chunks` / `file_state` only — weaker than JS; not a proposal change. Unintended test-gap. |
| C3-7 | FTS recall then drop after source change | `tests/readings.test.js` / `FTS recall returns stored readings and drops them after the source changes` | `rust/tests/readings.rs::fts_recall_returns_stored_readings_and_drops_them_after_hash_mismatch` | PASS | JS mutation appends `changed\n` (size+hash change); both prune. Matches proposal §3 hash-mismatch prune. |
| C3-8 | reject outside path and invalid range | `tests/readings.test.js` / `reading rejects paths outside the indexed project and invalid ranges` | `rust/tests/readings.rs::reading_rejects_paths_outside_the_indexed_project_and_invalid_ranges` | PASS | Rust also asserts `missing_file` / `not_a_file` |

---

## D — context / struct / impact / render / CLI (47) + eval (41)

| id | behavior | JS source | Rust test | verdict | notes |
|---|---|---|---|---|---|
| D-1 | default budget 1500 | `tests/context.test.js` / `the default budget is 1500 tokens` | `rust/tests/context.rs::the_default_budget_is_1500_tokens` | PASS | also `NEIGHBORS_PER_SEED=8`, `CONTENT_HEAD_LINES=12` |
| D-2 | exact symbol without FTS | `tests/context.test.js` / `an exact symbol name resolves without touching FTS` | `rust/tests/context.rs::an_exact_symbol_name_resolves_without_touching_fts` | PASS | both check `resolution=exact_symbol` only; neither spies FTS |
| D-3 | non-symbol query falls back to FTS | `tests/context.test.js` / `a non-symbol query falls back to FTS` | `rust/tests/context.rs::a_non_symbol_query_falls_back_to_fts` | PASS | unqualified `'return'` |
| D-4 | seeds carry depth-1 neighbours | `tests/context.test.js` / `seeds carry depth-1 neighbours` | `rust/tests/context.rs::seeds_carry_depth_1_neighbours` | PASS | neighbor `alpha` |
| D-5 | AMBIGUOUS neighbours dropped unless requested | `tests/context.test.js` / `AMBIGUOUS neighbours are dropped unless explicitly requested` | `rust/tests/context.rs::ambiguous_neighbours_are_dropped_unless_explicitly_requested` | PASS | |
| D-6 | unresolved inlined, never persisted | `tests/context.test.js` / `an unresolvable reference is inlined on the fly and never persisted` | `rust/tests/context.rs::an_unresolvable_reference_is_inlined_on_the_fly_and_never_persisted` | PASS | |
| D-7 | emitted JSON fits budget + truncated | `tests/context.test.js` / `the emitted JSON actually fits the budget and reports truncation` | `rust/tests/context.rs::the_emitted_json_actually_fits_the_budget_and_reports_truncation` | PASS | both `estimateTokens <= 400*1.15` and `truncated=true` |
| D-8 | unknown query empty packet | `tests/context.test.js` / `an unknown query returns an empty packet rather than throwing` | `rust/tests/context.rs::an_unknown_query_returns_an_empty_packet_rather_than_throwing` | PASS | |
| D-9 | context never invokes struct | `tests/context.test.js` / `context never invokes struct` | `rust/tests/context.rs::context_never_invokes_struct` | PASS | JS: no `from './struct.js'`; Rust: `src/context.rs` has no `r#struct` / `crate::struct` |
| D-10 | seed content truncated; restorable with fullContent | `tests/context.test.js` / `seed content is truncated by default and restorable with fullContent` | `rust/tests/context.rs::seed_content_is_truncated_by_default_and_restorable_with_full_content` | PASS | JS `export` note is a comment, not an assert |
| D-11 | short content untouched | `tests/context.test.js` / `short content is untouched and not flagged` | `rust/tests/context.rs::short_content_is_untouched_and_not_flagged` | PASS | |
| D-12 | Rust symbol returns only its function body | `tests/context.test.js` / `a Rust symbol returns only its function body, not the rest of a large file` | `rust/tests/context.rs::a_rust_symbol_returns_only_its_function_body_not_the_rest_of_a_large_file` | PASS | free fn `wanted`; proposal §4 extras do not replace this row |
| D-13 | struct constants | `tests/struct.test.js` / `constants match the spec` | `rust/tests/struct.rs::constants_match_the_spec` | PASS | also `UNBOUNDED_SCAN_FILE_LIMIT=2000` |
| D-14 | malformed pattern caught by preflight | `tests/struct.test.js` / `a malformed pattern is caught by the pre-flight, not by the exit code` | `rust/tests/struct.rs::a_malformed_pattern_is_caught_by_the_pre_flight_not_by_the_exit_code` | PASS | `parse_failed` + detail.pattern/lang |
| D-15 | valid pattern passes preflight | `tests/struct.test.js` / `a valid pattern passes the pre-flight` | `rust/tests/struct.rs::a_valid_pattern_passes_the_pre_flight` | PASS | |
| D-16 | zero matches is empty, not parse_failed | `tests/struct.test.js` / `zero matches is a clean empty result, never parse_failed` | `rust/tests/struct.rs::zero_matches_is_a_clean_empty_result_never_parse_failed` | PASS | |
| D-17 | matches 1-indexed | `tests/struct.test.js` / `matches are returned with 1-indexed lines` | `rust/tests/struct.rs::matches_are_returned_with_1_indexed_lines` | PASS | |
| D-18 | few malformed JSON lines skipped | `tests/struct.test.js` / `a few malformed JSON lines are skipped and counted` | `rust/tests/struct.rs::a_few_malformed_json_lines_are_skipped_and_counted` | PASS | 19/1 |
| D-19 | >10% malformed aborts this query | `tests/struct.test.js` / `more than 10% malformed aborts THIS query only` | `rust/tests/struct.rs::more_than_10_percent_malformed_aborts_this_query_only` | PASS | `run_aborted_malformed` 2/10 |
| D-20 | containmentJoin smallest chunk | `tests/struct.test.js` / `containmentJoin picks the smallest chunk that contains the match` | `rust/tests/struct.rs::containment_join_picks_the_smallest_chunk_that_contains_the_match` | PASS | `go` wins over `Beta` |
| D-21 | containmentJoin null when no chunk | `tests/struct.test.js` / `containmentJoin returns null when no chunk contains the match` | `rust/tests/struct.rs::containment_join_returns_null_when_no_chunk_contains_the_match` | PASS | |
| D-22 | MAX_NEIGHBORS cap + staleness | `tests/struct.test.js` / `structCommand attaches at most MAX_NEIGHBORS neighbours and reports staleness` | `rust/tests/struct.rs::struct_command_attaches_at_most_max_neighbors_neighbours_and_reports_staleness` | PASS | |
| D-23 | structCommand surfaces parse_failed | `tests/struct.test.js` / `structCommand surfaces parse_failed as a structured error and runs nothing` | `rust/tests/struct.rs::struct_command_surfaces_parse_failed_as_a_structured_error_and_runs_nothing` | PASS | `toJSON`/`to_json` `.error` |
| D-24 | unglobbed large scan refused; hint `-g` | `tests/struct.test.js` / `an unglobbed scan of a large project is refused with actionable advice` | `rust/tests/struct.rs::an_unglobbed_scan_of_a_large_project_is_refused_with_actionable_advice` | PASS | indexed_files=12, limit=10 |
| D-25 | glob narrows the same scan | `tests/struct.test.js` / `the same scan succeeds once a glob narrows it` | `rust/tests/struct.rs::the_same_scan_succeeds_once_a_glob_narrows_it` | PASS | |
| D-26 | default depth 3 | `tests/impact.test.js` / `the default depth is 3` | `rust/tests/impact.rs::the_default_depth_is_3` | PASS | |
| D-27 | dependents with hop distance | `tests/impact.test.js` / `dependents are returned with their hop distance` | `rust/tests/impact.rs::dependents_are_returned_with_their_hop_distance` | PASS | `{c:1,b:2,a:3}` |
| D-28 | depth respected | `tests/impact.test.js` / `depth is respected` | `rust/tests/impact.rs::depth_is_respected` | PASS | `['c']` |
| D-29 | no dependents → empty list | `tests/impact.test.js` / `a symbol with no dependents returns an empty list, not an error` | `rust/tests/impact.rs::a_symbol_with_no_dependents_returns_an_empty_list_not_an_error` | PASS | |
| D-30 | unknown symbol zero seeds | `tests/impact.test.js` / `an unknown symbol reports zero seeds without throwing` | `rust/tests/impact.rs::an_unknown_symbol_reports_zero_seeds_without_throwing` | PASS | |
| D-31 | ambiguous symbol seeds every match | `tests/impact.test.js` / `an ambiguous symbol seeds from every matching chunk` | `rust/tests/impact.rs::an_ambiguous_symbol_seeds_from_every_matching_chunk` | PASS | seed_count=2 |
| D-32 | unresolved inlined, nothing persisted | `tests/impact.test.js` / `unresolved references are inlined on the fly and nothing is persisted` | `rust/tests/impact.rs::unresolved_references_are_inlined_on_the_fly_and_nothing_is_persisted` | PASS | |
| D-33 | packet reports index staleness | `tests/impact.test.js` / `the packet reports index staleness` | `rust/tests/impact.rs::the_packet_reports_index_staleness` | PASS | `false` |
| D-34 | comma-separated symbols, min hop | `tests/impact.test.js` / `symbol accepts a comma-separated batch and merges dependents at min hop` | `rust/tests/impact.rs::symbol_accepts_a_comma_separated_batch_and_merges_dependents_at_min_hop` | PASS | `{b:1,a:2}` |
| D-35 | parseFormat json/lean case-insensitive | `tests/render.test.js` / `parseFormat accepts json and lean case-insensitively and rejects anything else` | `rust/tests/render.rs::parse_format_accepts_json_and_lean_case_insensitively_and_rejects_anything_else` | PASS | yaml → null |
| D-36 | lean impact lists hops, drops chunk_id | `tests/render.test.js` / `lean impact output lists every dependent with its hop and drops the stored chunk_id` | `rust/tests/render.rs::lean_impact_output_lists_every_dependent_with_its_hop_and_drops_the_stored_chunk_id` | PASS | |
| D-37 | lean smaller than json on three verbs | `tests/render.test.js` / `lean is smaller than json for the same payload on all three verbs` | `rust/tests/render.rs::lean_is_smaller_than_json_for_the_same_payload_on_all_three_verbs` | PASS | |
| D-38 | lean context neighbours one per line | `tests/render.test.js` / `lean context keeps neighbours and unresolved refs one per line` | `rust/tests/render.rs::lean_context_keeps_neighbours_and_unresolved_refs_one_per_line` | PASS | |
| D-39 | lean struct one row per match | `tests/render.test.js` / `lean struct emits one row per match with the enclosing symbol` | `rust/tests/render.rs::lean_struct_emits_one_row_per_match_with_the_enclosing_symbol` | PASS | |
| D-40 | unknown command / json fall through | `tests/render.test.js` / `unknown commands and json format fall through to the JSON contract` | `rust/tests/render.rs::unknown_commands_and_json_format_fall_through_to_the_json_contract` | PASS | |
| D-41 | lean reading provenance + stored content | `tests/render.test.js` / `lean reading output identifies cache provenance and keeps stored content` | `rust/tests/render.rs::lean_reading_output_identifies_cache_provenance_and_keeps_stored_content` | SUPERSEDED-by-proposal | JS header `source=store reads=2` **and** `includes('fn work()')`. Rust receipt: `# read … content=receipt hash=82d25b9f72a6\n` and `!contains("fn work()")`; full mode still has body. Intended §1. Recall two-line header still matches. |
| D-42 | help explains instead of running | `tests/cli.test.js` / `asking a command for help explains it instead of running it` | `rust/tests/cli.rs::asking_a_command_for_help_explains_it_instead_of_running_it` | PASS | cache empty |
| D-43 | every spelling of help | `tests/cli.test.js` / `every spelling of help reaches the same usage, and none of them is an error` | `rust/tests/cli.rs::every_spelling_of_help_reaches_the_same_usage_and_none_of_them_is_an_error` | PASS | Rust adds struct/context/read/recall/status/projects |
| D-44 | usage documents every known command | `tests/cli.test.js` / `usage documents every command the dispatcher actually knows` | `rust/tests/cli.rs::usage_documents_every_command_the_dispatcher_actually_knows` | PASS | |
| D-45 | unknown command is failure, not usage | `tests/cli.test.js` / `an unknown command is still a failure, not usage` | `rust/tests/cli.rs::an_unknown_command_is_still_a_failure_not_usage` | PASS | |
| D-46 | index without --help still indexes | `tests/cli.test.js` / `index without --help still indexes, so the guard did not swallow the command` | `rust/tests/cli.rs::index_without_help_still_indexes_so_the_guard_did_not_swallow_the_command` | PASS | |
| D-47 | read persists; recall finds via FTS | `tests/cli.test.js` / `read persists a fragment and recall finds it through FTS` | `rust/tests/cli.rs::read_persists_a_fragment_and_recall_finds_it_through_fts` | SUPERSEDED-by-proposal | JS asserts first `filesystem`, second `store`, recall count 1. Rust also `content_mode=receipt` and no `content` field (proposal §1). Recall still finds the note. |

### D-48..D-88 — eval harness (out of rust crate)

Spec §6: these test `evals/*.mjs`, not the rust crate. Rust `tests/` has no eval/grade/agent-stream/run-agents files.

| id | behavior | JS source | Rust test | verdict | notes |
|---|---|---|---|---|---|
| D-48 | three arms named by spec | `tests/eval-harness.test.js` / `the three arms are exactly the ones the spec names` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-49 | metric set | `tests/eval-harness.test.js` / `the metric set includes what the V6 eval plan was missing` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-50 | every task has expected answer | `tests/eval-harness.test.js` / `every task declares a verifiable expected answer` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-51 | summarize per-arm + stop/go | `tests/eval-harness.test.js` / `summarize computes per-arm aggregates and the stop/go verdict` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-52 | stop when cort loses on tokens | `tests/eval-harness.test.js` / `summarize returns a stop verdict when cort loses on tokens` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-53 | graph task labels each symbol once | `tests/eval-harness.test.js` / `every graph task labels each symbol exactly once, at one distance` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-54 | answer contract is one text | `tests/grade.test.js` / `the answer contract is one text, so both arms are asked for the same shape` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-55 | complete answer scores one on both axes | `tests/grade.test.js` / `a complete answer scores one on both axes` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-56 | missing symbol costs coverage | `tests/grade.test.js` / `a missing symbol costs coverage but not precision` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-57 | invented symbol costs precision | `tests/grade.test.js` / `an invented symbol costs precision but not coverage` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-58 | naming twice neither helps nor hurts | `tests/grade.test.js` / `naming a symbol twice neither helps coverage nor hurts precision` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-59 | wrong distance ≠ wrong symbol | `tests/grade.test.js` / `the wrong distance is recorded without being confused with a wrong symbol` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-60 | no block is a failed cell | `tests/grade.test.js` / `an answer with no block at all is a failed cell, not a null metric` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-61 | only the last block counts | `tests/grade.test.js` / `only the last block counts, so a quoted example cannot pad the answer` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-62 | spacing/bullets do not change the answer | `tests/grade.test.js` / `spacing and stray bullets in the block do not change the answer` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-63 | line without distance is unplaced | `tests/grade.test.js` / `a line without a distance still names a symbol, and is marked as unplaced` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-64 | gate is the one the plan fixed | `tests/grade.test.js` / `the gate is the one the plan fixed in advance, not one tuned to a result` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-65 | turn cap still gradeable | `tests/grade.test.js` / `a cell that hit the turn cap can still be graded on what it answered` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-66 | ASCII ~1 token / 4 chars | `tests/agent-stream.test.js` / `an ASCII payload costs about a token every four characters` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-67 | CJK costs a whole token | `tests/agent-stream.test.js` / `a CJK character costs a whole token, so cct comments are not under-counted` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-68 | empty payload is zero tokens | `tests/agent-stream.test.js` / `the empty payload is zero tokens, not a fraction of one` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-69 | tool results are measured | `tests/agent-stream.test.js` / `tool results are measured, which is the metric three rounds recorded as null` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-70 | Read calls counted apart | `tests/agent-stream.test.js` / `Read calls are counted apart from the other tools` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-71 | every cort invocation kept | `tests/agent-stream.test.js` / `every cort invocation is kept so the arm can be proved to have used its own tool` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-72 | usage summed as earlier rounds | `tests/agent-stream.test.js` / `usage is summed the way the earlier rounds summed it` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-73 | denied tool call surfaced | `tests/agent-stream.test.js` / `a denied tool call is surfaced, because a leaking whitelist invalidates the cell` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-74 | stream with no result throws | `tests/agent-stream.test.js` / `a stream that never produced a result throws instead of reporting nulls` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-75 | result without usage throws | `tests/agent-stream.test.js` / `a result without usage throws rather than writing a null metric` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-76 | blank lines skipped | `tests/agent-stream.test.js` / `blank lines in the stream are skipped, not treated as corruption` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-77 | turn cap is a fact, not failure | `tests/agent-stream.test.js` / `the turn cap is reported as a fact about the cell, not as failure` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-78 | tool_result as blocks or string | `tests/agent-stream.test.js` / `tool_result content arrives as blocks as well as a bare string` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-79 | two arms, identical prompt | `tests/run-agents.test.js` / `the two arms are the experiment: identical prompt, different tools` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-80 | neither arm gets the other's tool | `tests/run-agents.test.js` / `neither arm is handed the tool that defines the other arm` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-81 | cort prompt command is whitelisted | `tests/run-agents.test.js` / `the command the prompt tells the cort arm to run is one the whitelist accepts` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-82 | cell runs in the venue | `tests/run-agents.test.js` / `the cell runs in the venue, because projectId is derived from the cwd` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-83 | transcript flags requested | `tests/run-agents.test.js` / `the transcript flags the earlier rounds lacked are all requested` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-84 | only allowed tools passed | `tests/run-agents.test.js` / `every tool the arm may use is passed, and nothing else` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-85 | isolated environment | `tests/run-agents.test.js` / `the environment is the isolated one, not the user configuration` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-86 | row carries every metric, none null | `tests/run-agents.test.js` / `a row carries every metric the gate reads, none of them null` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-87 | missing metric refuses the row | `tests/run-agents.test.js` / `a row refuses to be built from a transcript missing a metric` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |
| D-88 | denied tool call carried onto the row | `tests/run-agents.test.js` / `a denied tool call is carried onto the row, where a reader will see it` | MISSING | MISSING | eval harness, out of rust crate (spec §6) |

---

## Extra Rust tests (not §6 rows)

These do not get B/C/D ids. They are proposal or plan-gap coverage sitting beside the table.

- `rust/tests/readings.rs` — proposal §1–§3: receipt/full, `--content receipt`, `invalid_content_mode`, equal-length+mtime, hash-before-store, classify table, ENOENT/EIO/EMFILE/EACCES/ENOTDIR/NotFound, multi-candidate fail-closed, race retry, non-regular, trim 12 lines, `invalid_limit`.
- `rust/tests/context.rs` — proposal §4 TDD-1..8: `canonical_owner_*`, `rust_pack_rules_*`, `six_impls_*`, `qualified_query_for_the_sixth_run_is_still_exact`, `trait_default_method_body_via_content_full`, `trait_impl_collision_*`, `type_method_json_and_lean_*`, `parse_symbol_query_*`, `method_record_without_owner_*`, `rust_yml_change_*`.
- `rust/tests/render.rs` — `validation_error_lean_is_a_single_line_and_nulls_become_dash`, `error_envelope_to_json_uses_error_not_code`, `json_pretty_print_uses_two_space_indent_and_trailing_newline`.
- `rust/tests/cli.rs` — `unknown_format_*`, `cort_cache_dir_is_honoured`, `cli_canonicalizes_root_before_project_id_for`, `missing_query_and_missing_symbol_*`, `type_method_end_to_end_cli_json_and_lean`.
