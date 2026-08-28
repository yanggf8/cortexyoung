// Compact one-record-per-line rendering for agents: the JSON contract carries
// chunk_id = projectId:file_path:start_line, so every row re-sends a 64-char hash
// plus a path it already has. Lean keeps only what an agent acts on.
export const FORMAT = { JSON: 'json', LEAN: 'lean' };

export function parseFormat(raw) {
  const value = typeof raw === 'string' ? raw.toLowerCase() : 'json';
  if (value === 'lean') return FORMAT.LEAN;
  if (value === 'json') return FORMAT.JSON;
  return null;
}

function ref(row) {
  return `${row.file_path}:${row.start_line}`;
}

export function renderImpact(payload) {
  const lines = [];
  const seeds = payload.seeds ?? [];
  lines.push(`# impact ${payload.symbol} depth=${payload.depth} seeds=${seeds.length} dependents=${payload.dependent_count} stale=${payload.index_is_stale}`);
  for (const s of seeds) lines.push(`seed\t${s.file_path}:${s.start_line}`);
  for (const d of payload.dependents ?? []) {
    lines.push(`h${d.hop}\t${d.file_path}\t${d.symbol_name ?? '?'}\t${d.start_line}`);
  }
  for (const u of payload.unresolved ?? []) {
    lines.push(`unresolved\t${u.symbol}\t${u.rel_type}\t${u.confidence}`);
  }
  return `${lines.join('\n')}\n`;
}

export function renderStruct(payload) {
  const lines = [];
  lines.push(`# struct ${payload.pattern} lang=${payload.lang} matches=${payload.match_count} shown=${(payload.matches ?? []).length} truncated=${payload.truncated} stale=${payload.index_is_stale}`);
  for (const m of payload.matches ?? []) {
    const neighbors = (m.neighbors ?? [])
      .map((n) => `${n.direction[0]}${n.rel_type}:${n.symbol_name ?? n.file_path}`)
      .join(',');
    lines.push([
      `${m.file_path}:${m.start_line}`,
      m.symbol_name ?? '?',
      neighbors,
      String(m.text ?? '').replaceAll('\n', ' ').slice(0, 120),
    ].filter(Boolean).join('\t'));
  }
  return `${lines.join('\n')}\n`;
}

export function renderContext(payload) {
  const lines = [];
  lines.push(`# context ${payload.query} resolution=${payload.resolution} seeds=${payload.seed_count} truncated=${payload.truncated} stale=${payload.index_is_stale}`);
  for (const s of payload.seeds ?? []) {
    lines.push(`${ref(s)}\t${s.symbol_name ?? '?'}\t${s.chunk_type}`);
    for (const n of s.neighbors ?? []) {
      lines.push(`  ${n.direction[0]}${n.rel_type}\t${n.file_path}:${n.start_line}\t${n.symbol_name ?? '?'}\t${n.confidence[0]}`);
    }
    for (const u of s.unresolved ?? []) {
      lines.push(`  unresolved\t${u.rel_type}\t${u.symbol}\t${u.confidence}`);
    }
    if (typeof s.content === 'string' && s.content.length > 0) {
      lines.push('  {');
      for (const line of s.content.split('\n')) lines.push(`  ${line}`);
      lines.push('  }');
    }
  }
  return `${lines.join('\n')}\n`;
}

export function renderRead(payload) {
  return `# read ${payload.file_path}:${payload.start_line}-${payload.end_line} source=${payload.source} reads=${payload.read_count}\n${payload.content}\n`;
}

export function renderRecall(payload) {
  const lines = [`# recall ${payload.query} readings=${payload.reading_count} truncated_query=${payload.truncated_query}`];
  for (const reading of payload.readings ?? []) {
    lines.push(`${reading.file_path}:${reading.start_line}-${reading.end_line}\treads=${reading.read_count}`);
    if (reading.content) lines.push(reading.content);
  }
  return `${lines.join('\n')}\n`;
}

export function render(command, format, payload) {
  if (format !== FORMAT.LEAN) return `${JSON.stringify(payload, null, 2)}\n`;
  if (command === 'impact') return renderImpact(payload);
  if (command === 'struct') return renderStruct(payload);
  if (command === 'context') return renderContext(payload);
  if (command === 'read') return renderRead(payload);
  if (command === 'recall') return renderRecall(payload);
  return `${JSON.stringify(payload, null, 2)}\n`;
}
