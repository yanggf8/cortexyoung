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

## Decision

Do not add Hindsight's integration or memory service to `cort`. This repo's product remains
the offline caller-set CLI. The separate `claudecat` repository is the place for a Rust-only
memory experiment if we proceed. Its first contract should be narrower than automatic
transcript ingestion: a fact requires a scope, source, observed time, evidence pointer, and
supersession state; a recalled claim must be checked against its authoritative source before
being promoted to an instruction. The next paired run should test both agents on the same
frozen facts, a superseded fact, and a pasted but false agent report, grading answers and
source attribution separately. The current two probes do not establish that any new memory
system improves correctness.

Upstream evidence:

- Hindsight bank resolution: <https://github.com/vectorize-io/hindsight/blob/main/hindsight-integrations/coding-agents/src/core/bank.ts>
- Session write-back: <https://github.com/vectorize-io/hindsight/blob/main/hindsight-integrations/coding-agents/src/core/retain-hook.ts>
- Memory injection: <https://github.com/vectorize-io/hindsight/blob/main/hindsight-integrations/coding-agents/src/core/hook.ts>
- Claude Code memory scopes: <https://code.claude.com/docs/en/memory>
