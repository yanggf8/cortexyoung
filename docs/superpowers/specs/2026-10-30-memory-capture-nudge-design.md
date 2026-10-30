# Capture at the moment of saving: a PostToolUse nudge from private memory to the ledger

Date: 2026-10-30. Status: revised after K3.1 Pro design review (2026-10-30). Owner: cortexyoung (`cort hook-refresh`).

## Why

AGENTS.md's goal leaves "automatic conversation capture" open. Before designing it, the gap was
measured on this repository's own Claude Code transcripts (NUC11i5, 2026-10-27 to 2026-10-30, the
period since SessionStart recall shipped). For each genuine user message (tool results, system
reminders, compaction summaries and subagent hand-backs excluded), the agent's actions until the
next user message were classified:

| What the agent did after the user's message | Messages |
|---|---|
| `cort memory record`/`resolve` only | 32 |
| ledger and a private `memory/*.md` write | 9 |
| private `memory/*.md` write only | 7 |
| neither | 83 |

The 7 private-only turns are almost all project decisions or corrections the agent itself judged
worth keeping but put where no other machine or harness reads them: "no Turso replicas, centralized
only", "remove every sync", "no long-term paid plan", "two gh accounts, just switch", "this is
cortexyoung, not tpm". Some reached the ledger a day later (`turso replicas`, 2026-10-28); "no
long-term paid Turso plan" and the two 2026-10-30 corrections never did. The "neither" bucket is
mostly confirmations ("好", "下一項", "."), with a few decisions recorded late or not at all ("先止血，改成只推不拉").

What this says: the agent usually *notices* a decision; it puts it in the wrong place, or later.
The rule already exists ("A harness-private memory is for how one agent works, never the only copy
of a project fact", AGENTS.md). It is forgotten at the moment it matters. The moment the agent saves
a private memory is the most precise capture point there is: it has just judged the fact worth
keeping and holds the whole conversation.

## What

When Claude Code's `PostToolUse` fires `cort hook-refresh` on a `Write`/`Edit` of a harness-private
memory file whose frontmatter `metadata.type` is `project` or `feedback`, `hook-refresh` prints,
instead of `{}`, exactly one line of JSON followed by a newline:

```json
{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"<text>"}}
```

with `<text>` (`<file>` is the file name only, `<type>` is `project` or `feedback`):

> cort memory: you saved `<file>` (type `<type>`) to private memory, which no other machine or
> harness reads. If it states a project fact, decision or correction and you have not recorded it
> yet, also run `cort memory record --key <key> --claim <claim>` (cite `--source/--line` only when a
> line states it). Skip if it only describes how you work.

It is written raw on one line, through a dedicated path like `hook-recall`'s, never through the
generic `Format::Lean` rendering (which pretty-prints).

The agent decides and writes the claim. `hook-refresh` never writes to the ledger.

Verified live before writing this (2026-10-30, `claude -p` with a scratch `--settings` hook,
haiku): a `PostToolUse` hook's `hookSpecificOutput.additionalContext` reached the model, which
quoted the marker verbatim. The payload carried `tool_input.file_path`, `transcript_path` and
`session_id`.

## Rules

1. **Recognition.** Reuse `notes_kick::is_memory_edit` (a `.md` directly under
   `<claude projects>/<project>/memory/`). `MEMORY.md` (the index) never nudges. The type is the
   `type:` key inside the `metadata:` block (not `node_type:`), read from the first 8 KiB by
   `notes::frontmatter`, the same line-based parser that uploads notes, so capture and upload can
   never disagree about a file (no YAML library: an unquoted `: ` in a description must not hide
   the type). No memory file on this machine puts `type:` at the top level, so there is no
   fallback for it. Only `project` and `feedback` nudge; `user`,
   `reference`, missing or unreadable frontmatter stay silent. Measured on this machine's
   cortexyoung memory: 15 feedback, 6 project, 2 reference, 1 user.
2. **No "already recorded" suppression.** Every qualifying save nudges, including when the agent
   recorded the same fact moments earlier (9 of the 16 memory-writing turns measured above). An
   earlier draft suppressed the nudge when the transcript showed a `cort memory record` since the
   last user message; review found it would also suppress after a *failed* record (the transcript
   model keeps calls, not results) and would silence an unrelated second fact in the same turn.
   One redundant line is cheaper than either miss, and the hook reads no transcript at all.
3. **Configured only.** No memory configuration (`turso-memory.json`) means `record` cannot work:
   stay silent. Notes opt-in is not required.
4. **Claude Code only.** Codex and Kimi do not write these files; the branch never runs for them.
5. **Still never fails.** Exit 0 always; any error on this path (the file cannot be read, its
   frontmatter cannot be parsed) is silence, tagged. The index repair and
   the notes kick keep running exactly as now; the nudge is decided after them and does not change
   their outcomes.
6. **Measured.** The usage row gains `capture=<outcome>` from a closed vocabulary, never a value
   copied from the file (the row is uploaded as telemetry): `nudged_project`, `nudged_feedback`,
   `type_user`, `type_reference`, `type_other`, `no_type`, `read_failed`, `index_file`,
   `not_memory`, `not_configured`, `not_claude`. Uptake is
   then measured by rerunning the transcript classification above (to become an `evals` subcommand
   if it is kept): the share of memory-writing turns that end private-only, 7 of 16 (44%) before.

## Contract change

`hook-refresh` was "silent and always succeeds: exit 0, `{}` on stdout". It stays always-succeeds;
it is no longer always silent. Its doc comment and AGENTS.md's "Two hook events" paragraph change
to say: silent except for the one capture line, which only appears on a private-memory save.

## Not doing

- **Writing to the ledger automatically.** The ledger is append-only; not every private memory is
  a project fact (15 of 24 here are feedback, some about how one agent works).
- **Mining transcripts with a model** at Stop/PreCompact. Heavier, costs tokens in the background,
  and the measurement says the agent already identifies most decisions.
- **Nudging on user prompts** (`UserPromptSubmit` keyword matching). The earlier suggestion
  experiment had 409 searches and zero uptake; a nudge must arrive when the agent is already acting
  on the thing, which a memory save is.

## Risk

Nudges have failed here before (reindex suggestions ignored 6 of 6). The difference argued for is
timing and intent: this one fires only when the agent has just chosen to save the fact. If the
private-only share does not fall after two weeks of use, remove the branch.

## Budget

`hook-refresh` measured 23-37 ms when nothing changed. The added work on a memory save is one
small file read (the frontmatter only); it must be measured on the implementation
(debug build, this machine) and stated in the commit, per AGENTS.md's time-budget rule.
