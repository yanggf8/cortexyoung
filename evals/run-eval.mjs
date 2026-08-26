export const ARMS = ['rg+Read', 'ast-grep+Read', 'cort'];
export const METRICS = ['total_tokens', 'tool_return_tokens', 'turns', 'read_calls', 'stale_reads'];

function mean(xs) { return xs.length === 0 ? 0 : xs.reduce((a, b) => a + b, 0) / xs.length; }

export function summarize(results) {
  const byArm = {};
  for (const arm of ARMS) {
    const rows = results.filter((r) => r.arm === arm);
    if (rows.length === 0) continue;
    byArm[arm] = {
      runs: rows.length,
      success_rate: rows.filter((r) => r.success).length / rows.length,
      mean_total_tokens: mean(rows.map((r) => r.total_tokens)),
      mean_tool_return_tokens: mean(rows.map((r) => r.tool_return_tokens)),
      mean_turns: mean(rows.map((r) => r.turns)),
      mean_read_calls: mean(rows.map((r) => r.read_calls)),
      stale_reads: rows.reduce((a, r) => a + r.stale_reads, 0),
    };
  }
  const base = byArm['ast-grep+Read'];
  const cort = byArm['cort'];
  const beats = Boolean(base && cort
    && cort.mean_total_tokens < base.mean_total_tokens
    && cort.success_rate >= base.success_rate);
  return {
    by_arm: byArm,
    verdict: {
      cort_beats_ast_grep: beats,
      // Spec section 8: this is a gate, not a report.
      next_action: beats ? 'continue to deferred features' : 'STOP: do not add features until cort wins',
    },
  };
}
