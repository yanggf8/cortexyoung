export const ARMS = ['rg+Read', 'ast-grep+Read', 'cort'];
export const METRICS = ['total_tokens', 'tool_return_tokens', 'turns', 'read_calls', 'stale_reads'];

function mean(xs) { return xs.length === 0 ? null : xs.reduce((a, b) => a + b, 0) / xs.length; }

// Rounds 1-3 recorded tool_return_tokens/read_calls as null in all 30 cells and still printed a
// verdict, because mean()/sum() turn null into NaN and NaN is JSON-serialised as null. A gate that
// cannot see its own inputs is not a gate, so: nulls are counted, never averaged into a number,
// and the verdict refuses to compare on a metric that was not measured.
function collect(rows, key) {
  const values = [];
  let missing = 0;
  for (const r of rows) {
    const v = r[key];
    if (typeof v === 'number' && Number.isFinite(v)) values.push(v);
    else missing += 1;
  }
  return { values, missing };
}

export function summarize(results, { strict = false } = {}) {
  const byArm = {};
  for (const arm of ARMS) {
    const rows = results.filter((r) => r.arm === arm);
    if (rows.length === 0) continue;
    byArm[arm] = { runs: rows.length, success_rate: rows.filter((r) => r.success).length / rows.length };
    const missing = {};
    for (const [out, key] of [
      ['mean_total_tokens', 'total_tokens'],
      ['mean_tool_return_tokens', 'tool_return_tokens'],
      ['mean_turns', 'turns'],
      ['mean_read_calls', 'read_calls'],
    ]) {
      const { values, missing: m } = collect(rows, key);
      byArm[arm][out] = mean(values);
      if (m) missing[key] = m;
    }
    const stale = collect(rows, 'stale_reads');
    byArm[arm].stale_reads = stale.values.length === rows.length
      ? stale.values.reduce((a, b) => a + b, 0)
      : null;
    if (stale.missing) missing.stale_reads = stale.missing;
    byArm[arm].metrics_missing = missing;
    if (strict && Object.keys(missing).length > 0) {
      throw new Error(`${arm}: refusing to summarise unmeasured metrics: ${JSON.stringify(missing)}`);
    }
  }
  const base = byArm['ast-grep+Read'];
  const cort = byArm['cort'];
  const comparable = base && cort
    && typeof base.mean_total_tokens === 'number'
    && typeof cort.mean_total_tokens === 'number';
  const beats = Boolean(comparable
    && cort.mean_total_tokens < base.mean_total_tokens
    && cort.success_rate >= base.success_rate);
  return {
    by_arm: byArm,
    verdict: {
      cort_beats_ast_grep: beats,
      // Spec section 8: this is a gate, not a report. An unmeasured metric fails closed — it is
      // never read as "no difference".
      reason: comparable ? 'compared on mean_total_tokens + success_rate' : 'metric-missing: no comparison possible',
      next_action: beats ? 'continue to deferred features' : 'STOP: do not add features until cort wins',
    },
  };
}
