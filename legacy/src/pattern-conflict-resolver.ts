/**
 * Pattern Conflict Resolution for Express + Passport Projects
 * 
 * Handles projects with mixed authentication patterns by implementing
 * "most recent pattern wins" strategy with evidence-based fallback.
 */

import { EvidenceItem, ConfidenceResult } from './confidence-scoring.js';
import * as fs from 'fs';
import * as path from 'path';

export interface PatternConflict {
    patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection';
    conflictingEvidence: EvidenceItem[];
    primaryPattern: EvidenceItem;
    conflictingPatterns: EvidenceItem[];
    resolution: 'most-recent' | 'highest-confidence' | 'manual-override';
    resolvedPattern: EvidenceItem;
}

export interface ConflictResolutionResult {
    hasConflicts: boolean;
    conflicts: PatternConflict[];
    resolvedPatterns: EvidenceItem[];
    resolutionStrategy: string;
    confidence: number;
}

export class PatternConflictResolver {
    /**
     * Resolve conflicts in detected authentication patterns
     */
    public resolveConflicts(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): ConflictResolutionResult {
        if (evidence.length <= 1) {
            return this.createNoConflictResult(evidence);
        }

        const conflicts = this.detectConflicts(patternType, evidence);
        
        if (conflicts.length === 0) {
            return this.createNoConflictResult(evidence);
        }

        const resolvedPatterns = this.applyResolutionStrategy(conflicts, evidence);
        const confidence = this.calculateResolutionConfidence(resolvedPatterns, evidence);

        return {
            hasConflicts: true,
            conflicts,
            resolvedPatterns,
            resolutionStrategy: 'most-recent-wins',
            confidence
        };
    }

    /**
     * Detect conflicts between different authentication patterns
     */
    private detectConflicts(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): PatternConflict[] {
        const conflicts: PatternConflict[] = [];

        // Group evidence by pattern variations
        const patternGroups = this.groupEvidenceByPattern(patternType, evidence);

        // Detect conflicts between different pattern implementations
        for (const [groupKey, groupEvidence] of Object.entries(patternGroups)) {
            if (groupEvidence.length > 1) {
                // Multiple files with same pattern - check for variations
                const variations = this.identifyPatternVariations(groupEvidence);
                
                if (variations.length > 1) {
                    const conflict = this.createConflict(patternType, groupEvidence, variations);
                    conflicts.push(conflict);
                }
            }
        }

        // Detect conflicts between different pattern types in same category
        if (Object.keys(patternGroups).length > 1) {
            const allEvidence = Object.values(patternGroups).flat();
            const conflict = this.createTypeConflict(patternType, allEvidence);
            conflicts.push(conflict);
        }

        return conflicts;
    }

    /**
     * Group evidence by pattern implementation
     */
    private groupEvidenceByPattern(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): Record<string, EvidenceItem[]> {
        const groups: Record<string, EvidenceItem[]> = {};

        for (const item of evidence) {
            let groupKey: string;

            switch (patternType) {
                case 'initialize':
                    // Group by initialization method
                    groupKey = this.extractInitializePattern(item.pattern);
                    break;
                case 'authenticate':
                    // Group by authentication strategy
                    groupKey = this.extractAuthenticateStrategy(item.pattern);
                    break;
                case 'strategy':
                    // Group by strategy type
                    groupKey = this.extractStrategyType(item.pattern);
                    break;
                case 'route-protection':
                    // Group by protection method
                    groupKey = this.extractProtectionMethod(item.pattern);
                    break;
                default:
                    groupKey = 'unknown';
            }

            if (!groups[groupKey]) {
                groups[groupKey] = [];
            }
            groups[groupKey].push(item);
        }

        return groups;
    }

    /**
     * Extract initialization pattern key
     */
    private extractInitializePattern(pattern: string): string {
        if (pattern.includes('passport.initialize()')) {
            return 'passport-initialize';
        }
        if (pattern.includes('session()')) {
            return 'passport-session';
        }
        return 'unknown-init';
    }

    /**
     * Extract authentication strategy key
     */
    private extractAuthenticateStrategy(pattern: string): string {
        const strategyMatch = pattern.match(/passport\.authenticate\(['"]([^'"]+)['"]/);
        if (strategyMatch) {
            return `auth-${strategyMatch[1]}`;
        }
        return 'auth-unknown';
    }

    /**
     * Extract strategy type key
     */
    private extractStrategyType(pattern: string): string {
        if (pattern.includes('LocalStrategy')) {
            return 'local-strategy';
        }
        if (pattern.includes('JWTStrategy')) {
            return 'jwt-strategy';
        }
        if (pattern.includes('GoogleStrategy')) {
            return 'google-strategy';
        }
        if (pattern.includes('FacebookStrategy')) {
            return 'facebook-strategy';
        }
        return 'unknown-strategy';
    }

    /**
     * Extract protection method key
     */
    private extractProtectionMethod(pattern: string): string {
        if (pattern.includes('ensureAuthenticated')) {
            return 'ensure-authenticated';
        }
        if (pattern.includes('requireAuth')) {
            return 'require-auth';
        }
        if (pattern.includes('authenticate')) {
            return 'inline-authenticate';
        }
        return 'unknown-protection';
    }

    /**
     * Identify variations within pattern group
     */
    private identifyPatternVariations(evidence: EvidenceItem[]): EvidenceItem[] {
        const variations: EvidenceItem[] = [];
        const seen = new Set<string>();

        for (const item of evidence) {
            // Normalize pattern for comparison
            const normalized = this.normalizePattern(item.pattern);
            
            if (!seen.has(normalized)) {
                seen.add(normalized);
                variations.push(item);
            }
        }

        return variations;
    }

    /**
     * Normalize pattern for variation detection
     */
    private normalizePattern(pattern: string): string {
        return pattern
            .replace(/\s+/g, ' ')
            .replace(/['"`]/g, '"')
            .toLowerCase()
            .trim();
    }

    /**
     * Create conflict object for pattern variations
     */
    private createConflict(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[],
        variations: EvidenceItem[]
    ): PatternConflict {
        const mostRecent = this.findMostRecentPattern(evidence);
        const conflicting = evidence.filter(e => e !== mostRecent);

        return {
            patternType,
            conflictingEvidence: evidence,
            primaryPattern: mostRecent,
            conflictingPatterns: conflicting,
            resolution: 'most-recent',
            resolvedPattern: mostRecent
        };
    }

    /**
     * Create conflict object for different pattern types
     */
    private createTypeConflict(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): PatternConflict {
        const mostRecent = this.findMostRecentPattern(evidence);
        const conflicting = evidence.filter(e => e !== mostRecent);

        return {
            patternType,
            conflictingEvidence: evidence,
            primaryPattern: mostRecent,
            conflictingPatterns: conflicting,
            resolution: 'most-recent',
            resolvedPattern: mostRecent
        };
    }

    /**
     * Find most recently modified pattern based on file timestamps
     */
    private findMostRecentPattern(evidence: EvidenceItem[]): EvidenceItem {
        let mostRecent = evidence[0];
        let mostRecentTime = this.getFileModificationTime(mostRecent.file);

        for (const item of evidence.slice(1)) {
            const itemTime = this.getFileModificationTime(item.file);
            
            if (itemTime > mostRecentTime) {
                mostRecent = item;
                mostRecentTime = itemTime;
            }
        }

        return mostRecent;
    }

    /**
     * Get file modification time
     */
    private getFileModificationTime(filePath: string): number {
        try {
            const stats = fs.statSync(filePath);
            return stats.mtime.getTime();
        } catch (error) {
            // If file doesn't exist or can't be accessed, use current time
            return Date.now();
        }
    }

    /**
     * Apply resolution strategy to conflicts
     */
    private applyResolutionStrategy(
        conflicts: PatternConflict[],
        allEvidence: EvidenceItem[]
    ): EvidenceItem[] {
        const resolved: EvidenceItem[] = [];
        const conflictedFiles = new Set<string>();

        // Track files involved in conflicts
        for (const conflict of conflicts) {
            for (const evidence of conflict.conflictingEvidence) {
                conflictedFiles.add(evidence.file);
            }
            
            // Add resolved pattern
            resolved.push(conflict.resolvedPattern);
        }

        // Add non-conflicted evidence
        for (const item of allEvidence) {
            if (!conflictedFiles.has(item.file)) {
                resolved.push(item);
            }
        }

        return resolved;
    }

    /**
     * Calculate confidence after conflict resolution
     */
    private calculateResolutionConfidence(
        resolvedPatterns: EvidenceItem[],
        originalEvidence: EvidenceItem[]
    ): number {
        const conflictRatio = 1 - (resolvedPatterns.length / originalEvidence.length);
        const baseConfidence = 0.8; // Base confidence for resolution
        
        // Reduce confidence based on conflict severity
        const conflictPenalty = conflictRatio * 0.3;
        
        return Math.max(0.5, baseConfidence - conflictPenalty);
    }

    /**
     * Create result for no conflicts
     */
    private createNoConflictResult(evidence: EvidenceItem[]): ConflictResolutionResult {
        return {
            hasConflicts: false,
            conflicts: [],
            resolvedPatterns: evidence,
            resolutionStrategy: 'no-conflicts',
            confidence: 1.0
        };
    }

    /**
     * Generate human-readable conflict summary
     */
    public generateConflictSummary(result: ConflictResolutionResult): string {
        if (!result.hasConflicts) {
            return 'No pattern conflicts detected. All evidence is consistent.';
        }

        const conflictCount = result.conflicts.length;
        const resolvedCount = result.resolvedPatterns.length;
        const confidence = Math.round(result.confidence * 100);

        let summary = `Resolved ${conflictCount} pattern conflict(s) using ${result.resolutionStrategy} strategy. `;
        summary += `Final patterns: ${resolvedCount} items with ${confidence}% confidence. `;

        // Add conflict details
        for (const conflict of result.conflicts) {
            const conflictingFiles = conflict.conflictingPatterns.map(p => path.basename(p.file)).join(', ');
            const resolvedFile = path.basename(conflict.resolvedPattern.file);
            
            summary += `${conflict.patternType}: chose ${resolvedFile} over ${conflictingFiles}. `;
        }

        return summary.trim();
    }
}

// Example usage and testing
export function testConflictResolution() {
    const resolver = new PatternConflictResolver();

    // Test case: Conflicting authentication strategies
    const conflictingEvidence: EvidenceItem[] = [
        {
            file: 'src/auth/local.ts',
            line: 15,
            pattern: 'passport.use(new LocalStrategy(...))',
            matchQuality: 'exact',
            context: 'passport.use(new LocalStrategy(verify));'
        },
        {
            file: 'src/auth/jwt.ts',
            line: 20,
            pattern: 'passport.use(new JWTStrategy(...))',
            matchQuality: 'exact',
            context: 'passport.use(new JWTStrategy(options, verify));'
        },
        {
            file: 'src/auth/google.ts',
            line: 12,
            pattern: 'passport.use(new GoogleStrategy(...))',
            matchQuality: 'exact',
            context: 'passport.use(new GoogleStrategy(config, verify));'
        }
    ];

    console.log('🔧 Testing Pattern Conflict Resolution...\n');

    const result = resolver.resolveConflicts('strategy', conflictingEvidence);
    
    console.log('Conflict Resolution Results:');
    console.log(`  Has Conflicts: ${result.hasConflicts}`);
    console.log(`  Conflicts Found: ${result.conflicts.length}`);
    console.log(`  Resolved Patterns: ${result.resolvedPatterns.length}`);
    console.log(`  Strategy: ${result.resolutionStrategy}`);
    console.log(`  Confidence: ${Math.round(result.confidence * 100)}%`);
    console.log('');

    const summary = resolver.generateConflictSummary(result);
    console.log('Summary:', summary);
    console.log('');

    return result;
}