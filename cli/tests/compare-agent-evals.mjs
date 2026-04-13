import fs from 'node:fs';
import process from 'node:process';

function usage() {
  console.error('Usage: node cli/tests/compare-agent-evals.mjs <baseline.json> <candidate.json>');
  process.exit(1);
}

if (process.argv.length < 4) {
  usage();
}

function readResults(path) {
  const raw = fs.readFileSync(path, 'utf8');
  const data = JSON.parse(raw);
  if (!Array.isArray(data)) {
    throw new Error(`${path}: expected a JSON array`);
  }

  return data.map((row, index) => {
    if (!row.task_id) throw new Error(`${path}[${index}]: missing task_id`);
    return {
      task_id: String(row.task_id),
      system: String(row.system ?? ''),
      success: Boolean(row.success),
      judge_notes: String(row.judge_notes ?? ''),
      input_tokens: Number(row.input_tokens ?? 0),
      output_tokens: Number(row.output_tokens ?? 0),
      total_tokens: Number(row.total_tokens ?? 0),
      tool_calls: Number(row.tool_calls ?? 0),
      raw_file_reads: Number(row.raw_file_reads ?? 0),
      latency_ms: Number(row.latency_ms ?? 0),
      final_answer_chars: Number(row.final_answer_chars ?? 0),
    };
  });
}

function median(values) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  if (sorted.length % 2 === 1) return sorted[mid];
  return (sorted[mid - 1] + sorted[mid]) / 2;
}

function mean(values) {
  if (values.length === 0) return 0;
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

function summarize(rows) {
  const passed = rows.filter(row => row.success);
  return {
    tasks: rows.length,
    passed: passed.length,
    pass_rate: rows.length === 0 ? 0 : passed.length / rows.length,
    median_total_tokens: median(passed.map(row => row.total_tokens)),
    mean_total_tokens: mean(passed.map(row => row.total_tokens)),
    median_tool_calls: median(passed.map(row => row.tool_calls)),
    median_raw_file_reads: median(passed.map(row => row.raw_file_reads)),
    median_latency_ms: median(passed.map(row => row.latency_ms)),
    tokens_per_success: passed.length === 0 ? 0 : passed.reduce((sum, row) => sum + row.total_tokens, 0) / passed.length,
  };
}

function pairedPassedRows(leftRows, rightRows) {
  const leftByTask = new Map(leftRows.map(row => [row.task_id, row]));
  const rightByTask = new Map(rightRows.map(row => [row.task_id, row]));
  const paired = [];

  for (const [taskId, left] of leftByTask.entries()) {
    const right = rightByTask.get(taskId);
    if (!right) continue;
    if (!left.success || !right.success) continue;
    paired.push({ task_id: taskId, left, right });
  }

  return paired;
}

function pctDelta(from, to) {
  if (from === 0) return 0;
  return ((to - from) / from) * 100;
}

function printSummary(label, summary) {
  console.log(label);
  console.log(`  tasks: ${summary.tasks}`);
  console.log(`  passed: ${summary.passed}`);
  console.log(`  pass_rate: ${(summary.pass_rate * 100).toFixed(1)}%`);
  console.log(`  median_total_tokens: ${summary.median_total_tokens.toFixed(1)}`);
  console.log(`  mean_total_tokens: ${summary.mean_total_tokens.toFixed(1)}`);
  console.log(`  median_tool_calls: ${summary.median_tool_calls.toFixed(1)}`);
  console.log(`  median_raw_file_reads: ${summary.median_raw_file_reads.toFixed(1)}`);
  console.log(`  median_latency_ms: ${summary.median_latency_ms.toFixed(1)}`);
  console.log(`  tokens_per_success: ${summary.tokens_per_success.toFixed(1)}`);
}

const baselinePath = process.argv[2];
const candidatePath = process.argv[3];

const baselineRows = readResults(baselinePath);
const candidateRows = readResults(candidatePath);

const baselineSummary = summarize(baselineRows);
const candidateSummary = summarize(candidateRows);
const paired = pairedPassedRows(baselineRows, candidateRows);

printSummary('Baseline', baselineSummary);
console.log('');
printSummary('Candidate', candidateSummary);
console.log('');

console.log(`Matched passed tasks: ${paired.length}`);

if (paired.length === 0) {
  console.log('No matched passed tasks. Cannot compute a valid token-saving comparison.');
  process.exit(0);
}

const tokenDeltas = paired.map(pair => pair.right.total_tokens - pair.left.total_tokens);
const rawReadDeltas = paired.map(pair => pair.right.raw_file_reads - pair.left.raw_file_reads);
const toolCallDeltas = paired.map(pair => pair.right.tool_calls - pair.left.tool_calls);
const latencyDeltas = paired.map(pair => pair.right.latency_ms - pair.left.latency_ms);

const baselinePairedTokens = paired.map(pair => pair.left.total_tokens);
const candidatePairedTokens = paired.map(pair => pair.right.total_tokens);

console.log(`Median token delta: ${median(tokenDeltas).toFixed(1)}`);
console.log(`Mean token delta: ${mean(tokenDeltas).toFixed(1)}`);
console.log(`Median token delta %: ${pctDelta(median(baselinePairedTokens), median(candidatePairedTokens)).toFixed(1)}%`);
console.log(`Mean token delta %: ${pctDelta(mean(baselinePairedTokens), mean(candidatePairedTokens)).toFixed(1)}%`);
console.log(`Median raw_file_reads delta: ${median(rawReadDeltas).toFixed(1)}`);
console.log(`Median tool_calls delta: ${median(toolCallDeltas).toFixed(1)}`);
console.log(`Median latency delta ms: ${median(latencyDeltas).toFixed(1)}`);

console.log('');
console.log('Per-task deltas');
for (const pair of paired) {
  const tokenDelta = pair.right.total_tokens - pair.left.total_tokens;
  const tokenDeltaPct = pctDelta(pair.left.total_tokens, pair.right.total_tokens);
  console.log(
    `  ${pair.task_id}: ${pair.left.total_tokens} -> ${pair.right.total_tokens} tokens ` +
    `(${tokenDelta >= 0 ? '+' : ''}${tokenDelta.toFixed(1)}, ${tokenDeltaPct.toFixed(1)}%)`
  );
}
