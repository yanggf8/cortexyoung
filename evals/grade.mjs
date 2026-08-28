// Grades one agent answer against a task's labels.
//
// Free prose cannot be scored for precision: you can tell whether an expected name appears, but
// not which of the other identifiers in the paragraph were meant as an answer. So both arms are
// asked, in identical words, to end with a machine-readable block. The tool whitelist is the only
// thing that differs between arms; this contract is appended to every prompt.

export const ANSWER_CONTRACT = [
  'End your reply with a fenced block in exactly this form, one line per function,',
  'the symbol name and its hop distance from the seed separated by a tab:',
  '',
  '```answer',
  'symbolName\t1',
  'otherSymbol\t2',
  '```',
  '',
  'List every function you found and nothing else. If you are unsure of a distance, still list',
  'the symbol.',
].join('\n');

// Fixed before any cell ran, so a disappointing number cannot move it afterwards.
export const GATE = { coverage: 0.9, precision: 0.7 };

const BLOCK = /```answer\s*\n([\s\S]*?)```/g;

function lastBlock(text) {
  let found = null;
  for (const m of String(text ?? '').matchAll(BLOCK)) found = m[1];
  return found;
}

function parseBlock(body) {
  const out = [];
  for (const raw of body.split('\n')) {
    const line = raw.replace(/^\s*[-*]\s*/, '').trim();
    if (line === '') continue;
    const m = line.match(/^([A-Za-z_$][\w$]*)(?:\s+h?(\d+))?\s*$/);
    if (!m) continue;
    out.push({ symbol: m[1], hop: m[2] === undefined ? null : Number(m[2]) });
  }
  return out;
}

function ratio(n, d) {
  return d === 0 ? 0 : Number((n / d).toFixed(3));
}

export function gradeAnswer(answerText, task) {
  const body = lastBlock(answerText);
  const rows = body === null ? [] : parseBlock(body);

  // First mention wins, so repeating a name cannot inflate or deflate either score.
  const said = new Map();
  for (const r of rows) if (!said.has(r.symbol)) said.set(r.symbol, r.hop);

  const expected = task.expected_symbols;
  const hopOf = new Map();
  for (const [hop, syms] of Object.entries(task.by_hop ?? {})) {
    for (const s of syms) hopOf.set(s, Number(hop));
  }

  const answered = [...said.keys()];
  const covered = expected.filter((s) => said.has(s));
  const spurious = answered.filter((s) => !expected.includes(s));

  const wrongHop = covered
    .filter((s) => said.get(s) !== hopOf.get(s))
    .map((s) => ({ symbol: s, said: said.get(s), actual: hopOf.get(s) }));

  const coverage = ratio(covered.length, expected.length);
  const precision = ratio(covered.length, answered.length);

  return {
    answer_block: body !== null,
    answered_symbols: answered,
    covered_symbols: covered,
    spurious_symbols: spurious,
    coverage,
    precision,
    hop_accuracy: ratio(covered.length - wrongHop.length, covered.length),
    wrong_hop: wrongHop,
    success: coverage >= GATE.coverage && precision >= GATE.precision,
  };
}
