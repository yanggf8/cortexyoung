# Round 3 — post-ergonomics verification (reduced)

Venue: /home/yanggf/a/cct (unchanged labels from round 2, incl. corrected t2).
Cells: the three heaviest round-2 cells only — who-calls-batchdualai,
blast-radius-getcurrenttimeet, who-calls-createbacktestingstorage.
Arms: ast-grep+Read (gate baseline) and cort. rg arm skipped (not the gate
comparator) to keep machine load low per user instruction.
Concurrency: 2 waves x 3 agents (round 2 ran 15-wide and thrashed).

Changes under test (commit 8a..): context seed content head-truncated to
12 lines by default (--content full restores); impact --symbol accepts a
comma-separated batch merged at min hop.
