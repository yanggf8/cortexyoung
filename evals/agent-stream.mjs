// Parses one `claude -p --output-format stream-json --verbose` transcript into the row the
// stop/go gate needs. Rounds 1-3 recorded tool_return_tokens and read_calls as null and nobody
// noticed for three rounds, so every metric here is either measured or thrown over — never null.

// Same estimator for both arms, per the eval plan: ASCII is ~4 chars to the token, and a CJK
// character is its own token. cct carries Traditional Chinese comments, and dividing those by 4
// would under-price the payload of whichever arm reads source, flattering it.
export function estimateTokens(text) {
  const s = String(text ?? '');
  let ascii = 0;
  let wide = 0;
  for (const ch of s) {
    if (ch.codePointAt(0) < 128) ascii += 1;
    else wide += 1;
  }
  return Math.ceil(ascii / 4) + wide;
}

function toolResultText(content) {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    return content.map((b) => (typeof b === 'string' ? b : b?.text ?? '')).join('');
  }
  return '';
}

export function parseStream(ndjson) {
  const events = [];
  for (const line of String(ndjson).split('\n')) {
    const trimmed = line.trim();
    if (trimmed === '') continue;
    events.push(JSON.parse(trimmed));
  }

  const toolCalls = [];
  let returnBytes = 0;
  let returnTokens = 0;
  let result = null;

  for (const e of events) {
    if (e.type === 'assistant') {
      for (const block of e.message?.content ?? []) {
        if (block.type === 'tool_use') toolCalls.push({ name: block.name, input: block.input ?? {} });
      }
    } else if (e.type === 'user') {
      for (const block of e.message?.content ?? []) {
        if (block.type !== 'tool_result') continue;
        const text = toolResultText(block.content);
        returnBytes += Buffer.byteLength(text, 'utf8');
        returnTokens += estimateTokens(text);
      }
    } else if (e.type === 'result') {
      result = e;
    }
  }

  if (result === null) throw new Error('transcript has no result event: the cell did not finish');
  const usage = result.usage;
  if (!usage) throw new Error('result event carries no usage: refusing to record a null metric');

  const part = (name) => {
    const v = usage[name];
    if (typeof v !== 'number') throw new Error(`usage.${name} is not a number: refusing to record a null metric`);
    return v;
  };

  return {
    turns: result.num_turns,
    hit_turn_cap: result.subtype === 'error_max_turns',
    tool_calls: toolCalls,
    read_calls: toolCalls.filter((c) => c.name === 'Read').length,
    tool_return_tokens: returnTokens,
    tool_return_bytes: returnBytes,
    input_tokens: part('input_tokens'),
    cache_creation: part('cache_creation_input_tokens'),
    cache_read: part('cache_read_input_tokens'),
    output_tokens: part('output_tokens'),
    total_tokens: part('input_tokens') + part('cache_creation_input_tokens')
      + part('cache_read_input_tokens') + part('output_tokens'),
    permission_denials: result.permission_denials ?? [],
    cost_usd: result.total_cost_usd,
    session_id: result.session_id,
    answer_text: typeof result.result === 'string' ? result.result : '',
  };
}
