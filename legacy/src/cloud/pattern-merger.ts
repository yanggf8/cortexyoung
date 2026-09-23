import type { SnapshotRow } from './turso-client.js';
import type { ProjectContextInfo, ImplementationPatterns } from '../types/patterns.js';

type CategoryKey = keyof ImplementationPatterns;

function mergeEvidence(arrays: string[][]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const arr of arrays) {
    for (const item of arr) {
      const key = item.toLowerCase().trim();
      if (!seen.has(key)) {
        seen.add(key);
        result.push(item);
      }
    }
  }
  return result;
}

function recencyWeight(scannedAt: string): number {
  const ageMs = Date.now() - new Date(scannedAt).getTime();
  const ageDays = ageMs / (1000 * 60 * 60 * 24);
  // Half-life of 30 days: a 30-day-old scan scores ~0.5x
  return Math.pow(0.5, ageDays / 30);
}

function mergeCategory<K extends CategoryKey>(
  snapshots: SnapshotRow[],
  key: K,
): ImplementationPatterns[K] {
  const sorted = [...snapshots].sort((a, b) => {
    const scoreA = (a.context.implementationPatterns[key].confidence ?? 0) * recencyWeight(a.scannedAt);
    const scoreB = (b.context.implementationPatterns[key].confidence ?? 0) * recencyWeight(b.scannedAt);
    return scoreB - scoreA;
  });

  const winner = sorted[0].context.implementationPatterns[key];
  const allEvidence = snapshots.map((s) => s.context.implementationPatterns[key].evidence ?? []);

  return {
    ...winner,
    evidence: mergeEvidence(allEvidence),
  } as ImplementationPatterns[K];
}

export function mergeSnapshots(snapshots: SnapshotRow[]): ProjectContextInfo {
  if (snapshots.length === 0) {
    throw new Error('mergeSnapshots requires at least one snapshot');
  }

  const latest = snapshots[0]; // sorted DESC by scannedAt from getAllSnapshots

  if (snapshots.length === 1) {
    return { ...latest.context, lastUpdated: new Date().toISOString() };
  }

  const merged: ImplementationPatterns = {
    authentication: mergeCategory(snapshots, 'authentication'),
    apiResponses: mergeCategory(snapshots, 'apiResponses'),
    errorHandling: mergeCategory(snapshots, 'errorHandling'),
  };

  return {
    ...latest.context,
    implementationPatterns: merged,
    lastUpdated: new Date().toISOString(),
  };
}
