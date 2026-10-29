# AGENTS.md

Instructions for any coding agent working in this repository (Codex reads `AGENTS.md`; Claude Code reads `CLAUDE.md`, which is a symlink to this file so the two can never drift).

This repo builds `cort`: local code intelligence over `ast-grep` + SQLite, plus an explicit
cross-machine project memory client backed by TursoDB.

**The repository's long-term product goal is durable, source-checkable project memory for Claude Code
across sessions: an agent should recover the right project fact or correction, know its scope and
source, and detect when it was superseded.** `cort memory` now supplies an explicit reviewed
decision ledger across machines, and since 2026-10-27 Claude Code's `SessionStart` hook pushes its active
entries into each session; automatic conversation capture remains open.
The shipped code-intelligence CLI has its own measurable goal: make the caller-set enumeration an
agent already performs — and often gets wrong — cheap, checkable edge by edge, and able to say
whether the set is complete. The third clause was promoted from a property of
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
row -- `cort-evals recall-exp` re-derives the population, so quote it with its commit). Since
2026-09-17 `cort-evals gate-audit` cuts those refusals by the gate's own reasons: on this repo at
`d6ea1991`, 34 of 9,971 raw receiver edges attached -- 9,586 zero-candidate (the std/dependency
frontier by construction), 185 multi-candidate, 145 ownerless, and 21 binding-refused, which is
the type-directed-dispatch candidate population and the number that decides whether trait-dispatch
edges are ever proposed -- and on a
hub-dense venue like cct the boolean is still true
for 62 of 63 sampled seeds -- which is why the instruction is *read the rows*, not *watch the flag*. A
change that makes an answer cheaper to verify is on the main line; a feature that only makes answers
more numerous is not.

**`PreToolUse` is a thin trigger since 2026-10-25; cort's own judgment stays silent.** It is deployed by
`install.sh` in the same run as the skill and runs `cort hook-suggest`, which reads the payload and
records one row in local `usage.db` -- `fired` with the harness, model, tool and project, or
`no_payload` when stdin was not JSON or carried no (or a null) `tool_input`; a `fired` row then kicks
collection when a run is due. Since 2026-10-28, a configured external gate via `cort hook-gate`
may deny a `Bash` call only when it exits 75; all other results stay silent and allow the call.
Claude Code, Codex and Kimi all honoured that deny on `Bash` in a live run on 2026-10-29. The hook measures demand; it is not a
path by which ClaudeCat memory reaches an agent. Recording without speaking began on 2026-09-27; the
search suggestion and the `judge` behind it were removed on 2026-10-25 (T5/T6), and what survives of
that vocabulary in `rust/src/hook.rs` is `LEGACY_SUGGEST_OUTCOMES` and `SUGGEST_DECLINES`, kept only
so a census can still attribute the v≤5 rows the judging hook left behind. The dated lessons stand:
the delivery experiment followed 409 searches in skill-bearing sessions with zero `cort` calls, and
the rule behind the suggestion was one decision across plural parsers -- a second copy of a
*parser* was just code, a second copy of the *decision* made `cort-evals hook-probe`'s calibration
describe something other than what shipped. A hand-rolled approximation of that rule, tried on
2026-09-02, over-counted its own corpus by 48% and 4x on the two surfaces
(`docs/2026-09-02-hook-wiring-correction.md` §15, §16); `hook-probe` itself was removed on
2026-10-24.

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
computers. Schema-v1 rows carry no machine of their own, so once two machines wrote to one old
database those rows cannot be separated. Schema-v2 rows carry a per-row machine stamp; the
aggregate still warns when its file's origin differs from the current machine.
**Harness is which program ran, model is who answered, and machine is where** -- three dimensions,
and the harness total stays whole regardless of the other two (`hook_models_at` is a second lens,
never a split of the first).

**Delivery was disabled by default, including Kimi's deny, and since 2026-10-25 there is nothing
left to deliver.** The `CORT_SUGGEST` switch and Kimi's deny went out with the suggestion itself
(T5/T6). The bounded exception is worth remembering as history: Kimi's `PreToolUse` kept only
results whose `action` was `block` and discarded every allow-shaped one before the model saw it, so
a suggestion there would have arrived as a deny or not at all -- once per symbol per session, then
yield. Whether that deny actually changed what the agent did was never settled: two runs and one
uptake is not enough to quote it as established (§16).

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

**Two hook events, and the second one repairs rather than reports.** `PreToolUse` on `Bash` (and
Kimi's `Grep`) records one row in `usage.db` and, for a `fired` row, kicks collection, speaking
nothing to the agent unless a gate configured with `cort hook-gate` denies a `Bash` call;
`PostToolUse` on edit tools runs `cort index --incremental`. The second
exists because the first, while it still judged, could only ever *say* the index was behind and
that was measured at 19 re-index runs against 2,700+ fires in 90 days -- and because the staleness it compared was git
heads, so an edited-but-uncommitted tree read as fresh while its answers were already wrong.
`hook-refresh` never creates an index, gives up rather than wait on a busy database, and is silent
and exits 0 whatever happens. Both entries live in the same three files, told apart by their
subcommand rather than by position -- in Kimi's flat `[[hooks]]` array nothing else can tell them
apart. An install-then-uninstall cycle must hand every one of those files back byte for byte,
including comments that belong to other owners; that is a test, because it was a bug first
(`docs/2026-09-02-hook-wiring-correction.md` §17).

**A third event, recall: `SessionStart` runs `cort hook-recall` for Claude Code and Codex, `UserPromptSubmit` for Kimi** (Claude Code since 2026-10-27; Codex and Kimi payloads intercepted 2026-10-29). Agents did
not recall the ledger unprompted, so recall is pushed at session start, resume, clear and compaction:
the active entries for the project go into `additionalContext`, newest first, capped at 20 entries and
6000 bytes. It also lists other machines' harness notes for the project -- names and hosts
only, never bodies, capped at 15 and 2000 bytes, and labelled unreviewed. It reads the shared
database over the network in one pipeline (plus one ledger-only retry when the notes statements fail
with a SQL error and at least 200 ms remain) and never writes to it or takes a local lock. The
network deadline is 3800 ms from process start (`remote::RECALL_DEADLINE`); the network timeout
defaults to 3500 ms (`CORT_RECALL_TIMEOUT_MS`), is capped at 3700 ms, and is also limited to the
remaining deadline minus 100 ms. It was 2500/1500 ms until 2026-10-29, when the first read after an idle
spell timed out on three session starts in one afternoon while warm reads took about 300 ms.
With under 200 ms left, it sends no request. A watchdog exits the process 900 ms after the deadline
(socket timeouts overshoot it) and 300 ms more once the answer is being written, so the whole run
stays inside 5 s. On
`remote_unreachable`, `remote_blocked`, or `remote_timeout`, it injects a one-line notice to retry
`cort memory recall "<topic>"` when needed. It records one `usage.db` row on every normal return.
Kimi discards `SessionStart` output, so its `UserPromptSubmit` entry prints the same text as a `message` on
the first prompt of each session only (a local marker per session id under the memory data directory).
`settings::events_for` is the one place that says which harness gets which event.

**Both staleness checks are one-sided, and they lean opposite ways.** The head comparison
(`main.rs`, what `hook-suggest` read while it still judged) missed an edited-but-uncommitted tree;
the content comparison (`staleness.rs`, the `index_is_stale` printed beside every `impact` row)
missed everything that moves HEAD *without* dirtying the tree -- `pull`, `checkout`, `rebase`,
`reset`, a teammate's commit -- because its candidate set was `git diff HEAD` and after a pull that
diff is empty. Fixed on 2026-09-03 by giving `git_candidates` the head the index was built from and
diffing against it too (`docs/2026-09-03-installer-dedup-and-attribution.md` §9). The half worth
remembering is the second one: `index --incremental` shared that candidate set, so it re-extracted
nothing and then stamped the new head on anyway, which means
the `PostToolUse` repair hook was not merely failing to repair a pulled tree -- on the next edit it
*erased the one signal* saying repair was needed. Measured on this repo at `70e228f5..0d51e55b`:
`index_is_stale: false` over 17 changed files and a 5-commit-old graph. **A candidate set that
cannot be narrowed honestly must widen to everything**, never quietly to nothing, which is what
`narrowed: false` now means for a missing git *and* for a stored head git cannot resolve.

**The index is a cache, and the query is the one who heals it.** Telling the agent to reindex was
measured worthless twice — stale suggestions ignored 6 of 6, then a copy-pasteable chained command
adopted once in four days — while the edit hook refused 905 rebuilds in 7 days it was forbidden to
attempt. So since 2026-09-14 `impact` runs the repair itself (`rust/src/heal.rs`)
before answering: milliseconds when an incremental suffices, the full 1-2s rebuild inline when it
does not, and a single-flight background rebuilder (`flock`, kernel-released on death, never a pid
file) for trees past `CORT_HEAL_MAX_FILES`. A foreground command a person chose to run has the
budget a hook never will; the `PreToolUse` hook reads no index at all -- since 2026-10-25 it only records
a row and kicks collection, and since 2026-10-28 hands a `Bash` payload to a configured gate under a
time limit -- and the edit hook only repairs an existing index. What the payload
owes the reader is unchanged: `self_healed`/`heal_deferred` name what the query did about
staleness, and every disclosure field still means what it meant — they describe the answer's
basis, they never assign the caller maintenance work. One arm of that contract had to learn to
speak on 2026-09-24: the never-create boundary no-opped **silently** on an empty index, so the husk
a sandboxed create left behind on travel-2026 answered `repair=rebuild_required` with no heal
marker and two separate agents read that as "the heal never fired" — the index was simply never
bootstrapped on that machine. That arm now answers
`heal_deferred: "empty_index_never_creates"`; the bootstrap lever remains an explicit `cort index`.

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

## Shared decision memory (cort)

**Ownership correction (2026-10-19): cortexyoung owns the Claude Code continuity goal and the
memory database client.** The 2026-10-18 recommendation confused shared database synchronization
with ownership of memory behavior; the user intended only the former. The dependency direction
is claudecat → cort. `cort memory` owns the reviewed claim ledger in the shared Turso database;
the local file-reading store (`cort read`/`recall`) was removed on 2026-10-23. See `docs/memory.md` for
configuration and `docs/2026-10-18-cross-agent-memory-baseline.md` for the correction.

The local `cort` index stays local. All measurement collection is cort's (user, 2026-10-27):
`cort measure`, kicked in the background from cort's own hooks, uploads to cort's own database;
ClaudeCat collects nothing, and its `claudecat telemetry sync` is retired.
`cort measure report` includes a gate section with hook handoff counts and latency by host and harness.

**Use the ledger in this repo; do not wait to be asked.** The product is only evidence if its own
developers lean on it, and on 2026-10-27 an agent kept a next-day plan in its harness-private memory
alone, where no other machine or harness can read it, until the user asked why.
- **Recall** with `cort memory recall "<topic>"` at the start of work, after a compaction, and
  before asserting a project fact (a decision, a machine's state, what was retired). A hit whose
  `evidence_status` is not `source_unchanged` is a lead to re-check, not a fact.
- **Record** with `cort memory record --key <key> --claim <claim> --source <repo file> --line <n>`
  when a decision is made or changed, when the user corrects a working assumption, and when a
  deployment changes what a machine runs. Cite the line that states it (a doc, plan or code line);
  if no line states it yet, write and commit that line first (`record` refuses a source that is
  untracked or dirty at HEAD). Replace the sole active claim with `--replace`, or pass
  `--supersedes <id>` (a unique lowercase hex prefix of at least 8 characters works); repeat
  `--supersedes` for every active id to resolve a conflict.
- A harness-private memory is for how one agent works, never the only copy of a project fact.

**Harness notes are a second, unreviewed table beside the ledger** (2026-10-27). `cort memory notes
upload` copies Claude Code's `projects/*/memory/*.md` into `harness_notes` in the same Turso database,
for projects opted in per machine with `cort memory notes opt-in`; `list`/`show` read other
machines' notes. They are never claims: nothing checks them against source. `hook-refresh` on note
edits and the interval kick run `notes upload --background` without making hooks wait; results go to
`memory-upload.log` in the memory data directory. The secret screen in
`rust/src/notes.rs` is coarse and fail-closed -- tuned on this machine's 680 real notes, where it
first dropped 605 (frontmatter `originSessionId`, `sk-` inside slugs, `auth.rs::x` paths) and now
drops one -- so read `upload --dry-run` before the first real upload on a machine. Tombstones come only
from a memory directory read to the end. `notes-uploaded.json` tracks uploads locally; if it is
missing or invalid, an upload reads this machine's rows from the shared database to recover tombstones.

## claudecat (separate repo)

**claudecat lives in its own repo ([yanggf8/claudecat](https://github.com/yanggf8/claudecat)) — do
not merge it in.** It was merged here once (`3cda23ea`, 2026-09-23) and backed out the same day:
two copies of one tool drift on the first commit to either side, the daily audit rows split
between two `CORT-AUDIT.md` files, and its symbols pollute this repo's cort index. Changes to
claudecat, its skill, and the cort-audit findings go to that repo; this repo ships `cort` and
its memory implementation. ClaudeCat may depend on `cort`; its memory implementation must not
become a second owner of the ledger.

<!-- claudecat:guardrails:begin -->
<!-- 技術決策 / Guardrails：每行一條，例如 `2D tilemap + Macroquad（禁 Python/3D）`、`插件一律裝在 Claude Code 內`。claudecat 只在此區不存在時建立，之後永不覆寫。 -->
<!-- claudecat:guardrails:end -->
