# Memory uptake: does Claude Code use what SessionStart pushes, and why

Status: design, 2026-10-27, revised after Codex's design review on the same day. Package P6 of the
harness-memory work. Not built yet.

## 1. Question

Since `774dbfa7` Claude Code's `SessionStart` hook injects the project's active ledger entries,
and since `eccc6820` it also lists other machines' harness notes. Delivery is proven: the
injection is in the context. Use is not. This package answers three questions, per exposure:

1. **Did the agent use it?**
2. **When it did not, was there a moment it should have (a miss)?** As opposed to an exposure the
   task never needed.
3. **Why, in either case?** From the agent's own words where the transcript kept them, labelled as
   inference where it did not.

The mechanical stage only produces candidates. "Used" and "missed" are reported only after a
reviewed judgment (§6), because no mechanical signal establishes either on its own.

Non-goals:
- No causal answer to "does injecting help". That needs a holdout (§9), deferred by decision on
  2026-10-27.
- No change to what is injected.
- No product change in this package. The proposed `session`/`source` usage-row fields were cut:
  `telemetry.rs` syncs whole usage rows to the shared store, so a raw session id would leave the
  machine. A remote row could count injections but never measure uptake without that machine's
  transcript.

## 2. Evidence that already exists

Measured on this machine's transcripts before writing this:

- **The exposure is in the transcript as an attachment.**
  - Each SessionStart that added context is recorded as an `attachment` entry with
    `attachment.type == "hook_additional_context"` and `attachment.hookEvent == "SessionStart"`.
  - `attachment.content` is an array of strings, one per hook, and other hooks share it.
  - Ours is the element beginning with `hook_recall.rs`'s `HEADER` (`cort memory: local replica`).
  - Ledger lines read `- [<key>] <claim> (<source>:<line>, <evidence_status>)`. The last shown line
    may be cut mid-claim and end in ` ...` (`hook_recall.rs`, the byte budget). Such an entry is
    parsed as **partial**: its tokens count, but its claim is never treated as a whole fact.
  - Note lines follow `NOTES_HEADER` and read `- <note_id> [<host>] <file_name> (<updated_at>)`.
- **`hook_success` entries are not one per exposure.** A single compact event was seen with two
  `hook_success` records and one additional-context attachment. The exposure is therefore keyed on
  the attachment that carries our element. The source (startup / resume / compact / clear) is taken
  from the nearest preceding `hook_success` `hookName` (`SessionStart:<source>`) within the same
  event, and is `unknown` when none matches.
- **Everything after it is in the same file**, with timestamps: user turns, assistant text, tool
  calls and tool results.
- **Reasoning is partly kept.** In one long session, 531 of 2,497 `thinking` blocks carry text and
  the rest are redacted.
- **A session file outlives its context.** One `sessionId` spans many compactions and resumes, and
  each one injects again. After a compaction, the first user turn is the compaction summary.
- **Subagents** write `<session>/subagents/*.jsonl` and mark entries `isSidechain`. Neither gets
  SessionStart context, and both are excluded.
- **Volume**: 9 transcript files changed in the first hours after the hook went live. The first
  readable report needs several days.

## 3. Unit: the exposure window

An **exposure window** starts at an attachment that carries our element. It ends at the next
SessionStart attachment of any kind in the same file, or at the end of the file.

Each window records:
- its source;
- the project hash (from `cwd`);
- the injected entries (key, claim, source, line, evidence status, partial flag);
- the listed notes;
- whether anything was left out (`... N more`).

Counting rules:
- A `hook_success` with no matching attachment is counted as an **unconfirmed hook run**, never as
  an exposure.
- Counts are reported three ways: per window, per distinct session, and per distinct
  (session, entry). Repeated exposures of one continuing task then cannot inflate a rate.

Cut from the first pass: pre-deployment baseline windows and post-deployment unexposed windows.
Both are useful only as a comparison, and a comparison is what §9 would do properly.

## 4. Use candidates (mechanical)

- **U1 commands, by purpose.** Bash tool calls running `cort memory`, counted per kind and never
  summed into one rate:
  - consumption: `recall`, `notes show`, `notes list`;
  - state check: `status`;
  - contribution: `record`, `resolve`.
  Only consumption is a use candidate.
- **U2 token reuse.** It covers distinctive tokens extracted from the injected lines: hex runs of 7
  or more characters, `path:line` sources, backticked spans and note file names.
  - A token counts when it appears in assistant text or thinking inside the window.
  - It must be absent from everything visible before that point: all earlier entries in the file,
    including earlier windows and the compaction summary; the other elements of the same
    SessionStart attachment; and the project's `CLAUDE.md`/`AGENTS.md` as read at scan time (a
    stated limitation, since the files can drift). It must also be absent from every tool result
    and user turn of the window that precede it.
  - A token that appears first in a **tool-call input** is recorded separately, as `U2-lookup`. The
    agent may have taken it from the injection to look it up, or may be running an ordinary lookup.
  - U2 is token reuse, not use. It becomes use only through the judge.
- **U3 source read.** A `Read`, or a Bash `sed`/`cat`/`grep`/`rg` naming the `source` file of an
  injected entry. Reading a cited source is often the right verification step, so U3 is neither a
  use nor a miss on its own. It is context for the judge.

## 5. Miss candidates (mechanical)

- **M1 source read without prior reuse.** U3 on an entry with no earlier U1 consumption or U2 for
  it. It is weak: the judge decides whether the fact the agent sought was the injected one.
- **M2 private-memory read.** Any `Read` under `<claude projects>/*/memory/`. When the file is one
  of the window's listed notes, that fact is recorded.
- **M3 user correction.** It is a user turn whose own words match a correction needle list:
  `不是`, `不對`, `錯了`, `你忘了`, `之前說過`, `我們決定`, `記錯`, `no,`, `wrong`, `we decided`,
  `I told you`. Own words come from `demand::own_words`, which strips pasted reports. The list is
  versioned like `DEMAND_METHOD`.

Cut: M4 (contradiction by token), whose recall would be too low to be worth the code.

## 6. Judgment: private packets, a reviewer model, a human audit

The tool does not call a model. For each sampled window it writes a **packet** with numbered turns.
Each turn carries its speaker (`user`, `assistant_text`, `thinking`, `tool_call`, `tool_result`,
`injection`) and its text, truncated per turn. A packet holds:
- the injection;
- every user turn of the window, as own words, so the reviewer sees the task;
- the bounded event sequence. Every turn from the window start to the last U/M candidate, with tool
  results reduced to their first lines, capped at a total size, with elision marked.

Sampling is stratified, with strata and weights recorded in the report:
- all windows with U1 consumption;
- all windows with any U2, U2-lookup or M candidate;
- a seeded random sample of the rest.

The reviewer is Muse, or K3.1 Pro when Muse is out of quota, and never the model that wrote the
tool. The prompt is committed. For each window the reviewer answers:
- `used`: `explicit` | `implicit` | `no`;
- `opportunity`: `yes` | `no` | `unclear`, with the turn number;
- `reason`, from one closed list:
  - when used: `task_needed_fact`, `verifying_a_claim`, `asked_by_user`, `prompted_by_skill`,
    `used_unstated`;
  - when not used: `not_relevant`, `already_in_instructions`, `not_noticed`, `distrusted`,
    `stale_or_wrong`;
- `evidence`: a list of `{turn, speaker, quote}`, or the literal `inferred`.

The merge step validates every judgment:
- A quote must occur verbatim in **that turn**, from **that speaker**.
- Speaker rules:
  - reasons about the agent's motive (`task_needed_fact`, `verifying_a_claim`, `distrusted`,
    `stale_or_wrong`, `already_in_instructions`) need an `assistant_text` or `thinking` quote, or
    they become `inferred`;
  - `asked_by_user` needs a `user` quote;
  - `not_noticed` and `used_unstated` are always `inferred`, because absence has no quote.
- A reason outside the list, a quote that fails these checks, or a malformed row is rejected and
  counted.

The report gives reason counts split into observed (quoted) and inferred, per stratum and
weighted. A **human audit** of the accepted judgments follows: the user, or I in the user's
presence, read 10 of them, chosen at random with the seed recorded, against their packets and
record agree or disagree. The report prints the audit's agreement beside the reason table. Without
an audit, the reasons are not published as findings.

## 7. Interface and privacy boundary

`cort-evals memory-uptake`, in `evals/src/uptake.rs`. Transcript traversal is shared with
`demand.rs` by making its `jsonl_files` a crate-visible helper. The scanner skips `subagents/`
paths and `isSidechain` entries itself. `own_words` is reused for user turns.

```
cort-evals memory-uptake [--claude-dir DIR] [--since RFC3339] [--out FILE]
                         [--private DIR --sample N --seed S] [--judged FILE]
```

- **Default.** Write an **aggregate-only** report. It holds:
  - method version, machine id and source, scan bounds, and the seed;
  - window, distinct-session and distinct (session, entry) counts, by source;
  - U1 by purpose, U2 and U2-lookup, U3, and M1–M3 counts;
  - per-stratum sample sizes and weights;
  - after `--judged`: rejected-row counts, reason counts (observed and inferred), and audit
    agreement.

  It holds no transcript-derived text: no excerpts, quotes, entry keys or claims, note file names
  or ids, and no paths. Entries and notes appear only as opaque, salted hashes, with the salt kept
  in the private directory, so per-entry rates stay comparable across runs without being readable.
- **`--private DIR`** holds everything with text: packets, the window detail, the judged rows
  (accepted and rejected), and the salt. DIR is canonicalized and refused when it is inside any git
  worktree, so `git rev-parse --show-toplevel` must fail from it.
- **`--judged FILE`** reads the reviewer's JSONL from the private directory, validates it as in §6,
  and writes accepted and rejected rows back there. Only the counts go into the report.

Committed artefacts (`evals/runs/<date>-memory-uptake/`) are the aggregate report and the reviewer
prompt.

## 8. Delivery plan

Two delegations of about ten minutes each:

1. **Scanner and aggregate report.** Exposure extraction (attachment-keyed, with partial entries),
   windows, U1 by purpose, U2 and U2-lookup with the pre-exposure history rule, U3, M1–M3,
   distinct counts, the aggregate report, the privacy boundary of `--private`, and tests on
   synthetic transcripts only. No real transcript ever becomes a fixture.
2. **Sampling, packets and the judged merge.** The stratified sampler, the numbered-turn packets,
   the reviewer prompt, the `--judged` validation, and its tests.

The first real run follows after a few days of use, then a Muse judgment pass and the human audit.

## 9. Holdout (deferred)

Without a holdout the report cannot say that injecting changed anything, only how often what was
injected was used. If after a week the miss candidates are frequent or most reasons are
`not_noticed`, the next step is a deterministic per-session holdout: no injection when
`sha256(session_id) mod 5 == 0`, recorded locally as `hook: "held_out"`. That is a new member of
`RECALL_OUTCOMES` in `rust/src/hook.rs`, so the census learns it. A session never switches arm
across its compactions. Not built now.

## 10. Risks

- **U2 false positives.** These are limited by the pre-exposure history rule, by keeping lookups
  separate, and by the judge. U2 is never reported as use without judgment.
- **U2 false negatives.** A paraphrase leaves no token, so such uses surface only as the judge's
  `used_unstated`.
- **M3 low recall.** It is a candidate list for the judge, never a rate quoted alone.
- **Judge bias.** It is checked by the closed list, the turn- and speaker-bound quote validation,
  the observed/inferred split, and the human audit.
- **Privacy.** The report is aggregate-only, uses salted opaque ids, keeps text in a canonicalized
  directory outside any worktree, and never uses a real transcript as a fixture.
- **Small n.** Every rate is printed with its n, and a rate under n = 5 prints `-`.
