# Cross-agent memory baseline, 2026-10-18

Machine `44a8a38f0b05b9a8` (`etc-machine-id`), repo head `abe719cc`.
Claude Code 2.1.302 and Codex CLI 0.163.0. Each probe used a fresh headless session,
the same prompt for both agents, and requested no tool calls. Claude ran with `--tools ""`
and `--max-turns 1`; Codex ran with `--sandbox read-only --ephemeral --json`.
This is one run per cell, an availability check, not an accuracy rate.

| Probe | Claude Code | Codex |
|---|---|---|
| Prior correction about terse `部署安裝` | Recalled: the command targeted the current `cortexyoung` session directory, citing the auto-memory index | Answered `未知`, citing the absence of this correction from automatically loaded project instructions |
| Presence of `travel-2026` memory while in this repo | Recalled a separate auto-memory index entry about concurrent Codex work in that repo; also noticed a mention in project instructions | Saw the mention in `AGENTS.md` only |

The first prompt required each agent to rely only on context automatically loaded at session
start, to avoid tools and guessing, and to answer `未知` when evidence was insufficient. The
second used the same constraints and asked whether any durable `travel-2026` information was
visible. The second probe does not establish cross-project leakage on its own: `AGENTS.md`
already mentions that repo. It does establish that Claude Code loaded a separate machine-local
auto-memory index entry that Codex did not have.

The repo's `CLAUDE.md` is a symlink to `AGENTS.md`, so shared project instructions have one
source. Claude Code's auto-memory index is a second, machine-local source; on this machine it
has three topic entries, including one about another repo and one about a particular sandbox
failure. Codex has no corresponding Hindsight configuration on this machine. Hindsight itself
was not measured: no server, credentials, or running local LLM were available. The official
coding-agent package uses TypeScript/JavaScript hooks, and the published Hindsight server is
not a Rust implementation. Running it here would not test an all-Rust production design.

## Recommendation made on 2026-10-18 (corrected below)

The recommendation was: do not add Hindsight's integration or memory service to `cort`;
keep the shipped product as the offline caller-set CLI, and put any Rust-only memory experiment
in the separate `claudecat` repository. Its first contract should be narrower than automatic
transcript ingestion: a fact requires a scope, source, observed time, evidence pointer, and
supersession state; a recalled claim must be checked against its authoritative source before
being promoted to an instruction. The next paired run should test both agents on the same
frozen facts, a superseded fact, and a pasted but false agent report, grading answers and
source attribution separately. The current two probes do not establish that any new memory
system improves correctness.

## Ownership correction, 2026-10-19

The recommendation above was wrong. It treated a shared database and synchronization path as
though they determined which repository owns Claude Code's continuing memory. They do not.
The user intended database centralization, not a transfer of the memory product to claudecat.
The two probes measured what the agents could recall; they provided no evidence for assigning
the memory feature to claudecat. **Cortexyoung is the repository that should help Claude Code
retain and recover verified project knowledge across sessions.** Caller-set enumeration remains
a measured capability of today's `cort`; it is not the whole long-term product goal.

The boundary is functional: cortexyoung owns what is retained, how claims are scoped and
superseded, when a claim is recalled, how its source is checked, and whether the memory actually
improves later answers. The dependency direction is ClaudeCat → cort. The database connection,
sync replica, and reviewed claim logic belong in cort; ClaudeCat may call cort but does not own
a second memory database. Centralizing bytes elsewhere would not transfer these decisions.

`cort memory` now supplies an explicit reviewed decision ledger with the existing Turso table
format and a cort-owned local replica. `docs/memory.md` names the migration and verification
steps. Do not merge the claudecat repository here or claim that `cort read`/`recall` remembers
conversations: those commands validate stored source fragments. The new memory CLI still needs
the paired accuracy and attribution test described above. The two baseline probes do not
establish a benefit from the implemented path or from automatic capture.

Upstream evidence:

- Hindsight bank resolution: <https://github.com/vectorize-io/hindsight/blob/main/hindsight-integrations/coding-agents/src/core/bank.ts>
- Session write-back: <https://github.com/vectorize-io/hindsight/blob/main/hindsight-integrations/coding-agents/src/core/retain-hook.ts>
- Memory injection: <https://github.com/vectorize-io/hindsight/blob/main/hindsight-integrations/coding-agents/src/core/hook.ts>
- Claude Code memory scopes: <https://code.claude.com/docs/en/memory>
