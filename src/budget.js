export function estimateTokens(text) {
  return Math.ceil(String(text ?? '').length / 4);
}

export function applyBudget(items, budgetTokens, render) {
  const kept = [];
  let used = 0;
  for (const item of items) {
    const cost = estimateTokens(render(item));
    if (kept.length > 0 && used + cost > budgetTokens) {
      return { kept, truncated: true };
    }
    kept.push(item);
    used += cost;
  }
  return { kept, truncated: false };
}
