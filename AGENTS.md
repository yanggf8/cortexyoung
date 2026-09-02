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
singular: **parsing is per-harness and plural, the verdict is singular.** A shell line, Codex's
`["bash","-lc",…]` and Kimi's structured `Grep` fields are three different extractions and each gets
its own function; all three build a `Search` and all three hand it to the one `judge`. A second copy
of a *parser* is just code; a second copy of the *decision* makes `cort-evals hook-probe`'s
calibration describe something other than what ships, which is the only thing that number is for. So
`hook-probe` replays `judge` itself, and never reimplements it -- a hand-rolled approximation of the
rule was tried on 2026-09-02 and over-counted its own corpus by 48% and 4x on the two surfaces
(`docs/2026-09-02-hook-wiring-correction.md` §15, §16). `cort hook-install` owns the settings merge
for the same reason a `jq` pipeline would not: preserving other people's hooks, collapsing
duplicates, and refusing a file it cannot parse are logic, and logic needs tests -- one module per
dialect (`settings.rs` JSON, `settings_toml.rs` Codex, `settings_kimi.rs` Kimi), chosen by an
explicit `--format` since two of the three files are called `config.toml`. Recognition of our own
entry is a token test, never a suffix test -- anchoring it to the end of the command line is what
let `--status` report `wired: false` on a machine where the hook was firing, twice
(`docs/2026-09-02-hook-wiring-correction.md`).

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
