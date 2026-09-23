import type { ProjectContextInfo } from '../types/patterns.js';

const colors = {
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  red: '\x1b[31m',
  dim: '\x1b[2m',
  bold: '\x1b[1m',
  reset: '\x1b[0m',
} as const;

function confidenceColor(score: number): string {
  if (score >= 80) return colors.green;
  if (score >= 50) return colors.yellow;
  return colors.red;
}

function confidenceBar(score: number): string {
  const filled = Math.round(score / 10);
  const empty = 10 - filled;
  const color = confidenceColor(score);
  return `${color}${'█'.repeat(filled)}${'░'.repeat(empty)} ${score}%${colors.reset}`;
}

function formatPatternBlock(name: string, pattern: { confidence: number; evidence: string[]; [key: string]: unknown } | undefined): string {
  if (!pattern) return '';

  const lines: string[] = [];
  lines.push(`  ${colors.bold}${name}${colors.reset}  ${confidenceBar(pattern.confidence)}`);

  for (const [key, value] of Object.entries(pattern)) {
    if (key === 'confidence' || key === 'evidence') continue;
    if (typeof value === 'object' && value !== null) {
      // Nested object (e.g., errorResponse)
      for (const [subKey, subValue] of Object.entries(value as Record<string, unknown>)) {
        const displayValue = subValue === 'Unknown' || subValue === 0 ? `${colors.dim}unknown${colors.reset}` : String(subValue);
        lines.push(`    ${colors.dim}${subKey}:${colors.reset} ${displayValue}`);
      }
    } else {
      const displayValue = value === 'Unknown' ? `${colors.dim}unknown${colors.reset}` : String(value);
      lines.push(`    ${colors.dim}${key}:${colors.reset} ${displayValue}`);
    }
  }

  // Show evidence if available
  if (pattern.evidence.length > 0) {
    lines.push(`    ${colors.dim}evidence:${colors.reset}`);
    for (const e of pattern.evidence.slice(0, 3)) {
      lines.push(`      ${colors.dim}- ${e}${colors.reset}`);
    }
  }

  return lines.join('\n');
}

export function formatScanResult(context: ProjectContextInfo): string {
  const lines: string[] = [];

  lines.push(`${colors.bold}ClaudeCat Deep Analysis${colors.reset}`);
  lines.push(`${colors.dim}${'─'.repeat(50)}${colors.reset}`);

  // Project info
  lines.push(`  Project: ${context.projectType || 'unknown'}`);
  lines.push(`  Language: ${context.language || 'unknown'}`);
  lines.push(`  Framework: ${context.framework || 'unknown'}`);
  lines.push('');

  // Implementation patterns
  const impl = context.implementationPatterns;
  if (impl) {
    lines.push(`${colors.bold}Implementation Patterns${colors.reset}`);
    lines.push(`${colors.dim}${'─'.repeat(50)}${colors.reset}`);

    const authBlock = formatPatternBlock('Authentication', impl.authentication);
    if (authBlock) lines.push(authBlock, '');

    const apiBlock = formatPatternBlock('API Response', impl.apiResponses);
    if (apiBlock) lines.push(apiBlock, '');

    const errorBlock = formatPatternBlock('Error Handling', impl.errorHandling);
    if (errorBlock) lines.push(errorBlock, '');
  } else {
    lines.push(`  ${colors.dim}No implementation patterns detected${colors.reset}`);
  }

  return lines.join('\n');
}

export function formatStatus(context: ProjectContextInfo): string {
  const scanOutput = formatScanResult(context);
  const lines = [scanOutput];

  lines.push(`${colors.dim}${'─'.repeat(50)}${colors.reset}`);
  lines.push(`  ${colors.dim}Last analyzed: ${context.lastUpdated || 'never'}${colors.reset}`);

  return lines.join('\n');
}
