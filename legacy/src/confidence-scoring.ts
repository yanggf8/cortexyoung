/**
 * Evidence-Weighted Confidence Scoring for Passport Pattern Detection
 * 
 * Replaces the current overconfident scoring system with realistic confidence
 * based on actual evidence quality and pattern complexity.
 */

export interface EvidenceItem {
    file: string;
    line: number;
    pattern: string;
    matchQuality: 'exact' | 'partial' | 'inferred';
    context: string; // surrounding code context
}

export interface ConfidenceFactors {
    evidenceCount: number;      // How many files contain this pattern
    patternComplexity: number;  // How specific/unique is the pattern
    contextQuality: number;     // How clear is the surrounding code
    consistency: number;        // How consistent across files
    recency: number;           // How recent are the files
}

export interface ConfidenceResult {
    score: number;              // 0-100 confidence percentage
    level: 'low' | 'medium' | 'high' | 'very-high';
    factors: ConfidenceFactors;
    evidence: EvidenceItem[];
    reasoning: string;
    uncertainty: string[];     // List of uncertainty factors
}

export class PassportConfidenceScorer {
    /**
     * Calculate evidence-weighted confidence for a detected pattern
     */
    public calculateConfidence(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): ConfidenceResult {
        if (evidence.length === 0) {
            return this.createLowConfidenceResult('No evidence found', evidence);
        }

        const factors = this.calculateFactors(patternType, evidence);
        const score = this.computeWeightedScore(factors);
        const level = this.determineConfidenceLevel(score);
        const uncertainty = this.identifyUncertaintyFactors(factors, evidence);
        const reasoning = this.generateReasoning(patternType, factors, evidence);

        return {
            score: Math.round(score),
            level,
            factors,
            evidence,
            reasoning,
            uncertainty
        };
    }

    /**
     * Calculate individual confidence factors
     */
    private calculateFactors(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): ConfidenceFactors {
        return {
            evidenceCount: this.calculateEvidenceCount(evidence),
            patternComplexity: this.calculatePatternComplexity(patternType, evidence),
            contextQuality: this.calculateContextQuality(evidence),
            consistency: this.calculateConsistency(evidence),
            recency: this.calculateRecency(evidence)
        };
    }

    /**
     * Evidence count scoring (more evidence = higher confidence)
     */
    private calculateEvidenceCount(evidence: EvidenceItem[]): number {
        const uniqueFiles = new Set(evidence.map(e => e.file)).size;
        
        // Score based on number of files containing pattern
        if (uniqueFiles >= 3) return 1.0;      // Multiple files - very confident
        if (uniqueFiles === 2) return 0.8;     // Two files - good confidence
        if (uniqueFiles === 1) return 0.6;     // Single file - medium confidence
        return 0.0;                             // No files - no confidence
    }

    /**
     * Pattern complexity scoring (more specific patterns = higher confidence)
     */
    private calculatePatternComplexity(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        evidence: EvidenceItem[]
    ): number {
        // Different pattern types have different complexity levels
        const baseComplexity = {
            'initialize': 0.9,      // passport.initialize() is very specific
            'strategy': 0.85,       // passport.use(new Strategy()) is specific  
            'authenticate': 0.8,    // passport.authenticate() is common
            'route-protection': 0.7 // Could be other middleware too
        }[patternType];

        // Adjust based on evidence specificity
        const exactMatches = evidence.filter(e => e.matchQuality === 'exact').length;
        const totalEvidence = evidence.length;
        const exactRatio = exactMatches / totalEvidence;

        return baseComplexity * (0.5 + 0.5 * exactRatio);
    }

    /**
     * Context quality scoring (clear context = higher confidence)
     */
    private calculateContextQuality(evidence: EvidenceItem[]): number {
        let totalQuality = 0;

        for (const item of evidence) {
            let itemQuality = 0;

            // Check for clear Express/Passport context indicators
            const context = item.context.toLowerCase();
            
            if (context.includes('app.use') || context.includes('this.app.use')) {
                itemQuality += 0.3; // Clear Express app usage
            }
            
            if (context.includes('passport.')) {
                itemQuality += 0.3; // Clear Passport usage
            }
            
            if (context.includes('import') && context.includes('passport')) {
                itemQuality += 0.2; // Passport imported
            }
            
            if (context.includes('new') && context.includes('strategy')) {
                itemQuality += 0.2; // Strategy instantiation
            }

            totalQuality += Math.min(itemQuality, 1.0);
        }

        return evidence.length > 0 ? totalQuality / evidence.length : 0;
    }

    /**
     * Consistency scoring (same pattern across files = higher confidence)
     */
    private calculateConsistency(evidence: EvidenceItem[]): number {
        if (evidence.length <= 1) return 1.0; // Single evidence is "consistent"

        // Group evidence by pattern
        const patternGroups = evidence.reduce((acc, item) => {
            acc[item.pattern] = (acc[item.pattern] || 0) + 1;
            return acc;
        }, {} as Record<string, number>);

        // Calculate consistency as ratio of most common pattern
        const maxCount = Math.max(...Object.values(patternGroups));
        return maxCount / evidence.length;
    }

    /**
     * Recency scoring (recent files = higher confidence)
     */
    private calculateRecency(evidence: EvidenceItem[]): number {
        // This is simplified - in practice you'd check file modification times
        // For now, assume all evidence is recent
        return 1.0;
    }

    /**
     * Compute weighted confidence score from factors
     */
    private computeWeightedScore(factors: ConfidenceFactors): number {
        // Weighted combination of factors
        const weights = {
            evidenceCount: 0.25,    // 25% - How much evidence
            patternComplexity: 0.30, // 30% - How specific the pattern
            contextQuality: 0.25,   // 25% - How clear the context
            consistency: 0.15,      // 15% - How consistent across files
            recency: 0.05          // 5% - How recent the evidence
        };

        return (
            factors.evidenceCount * weights.evidenceCount +
            factors.patternComplexity * weights.patternComplexity +
            factors.contextQuality * weights.contextQuality +
            factors.consistency * weights.consistency +
            factors.recency * weights.recency
        ) * 100; // Convert to percentage
    }

    /**
     * Determine confidence level from score
     */
    private determineConfidenceLevel(score: number): 'low' | 'medium' | 'high' | 'very-high' {
        if (score >= 90) return 'very-high';
        if (score >= 75) return 'high';
        if (score >= 60) return 'medium';
        return 'low';
    }

    /**
     * Identify factors that create uncertainty
     */
    private identifyUncertaintyFactors(
        factors: ConfidenceFactors,
        evidence: EvidenceItem[]
    ): string[] {
        const uncertainty: string[] = [];

        if (factors.evidenceCount < 0.7) {
            uncertainty.push('Limited evidence - pattern found in few files');
        }

        if (factors.patternComplexity < 0.7) {
            uncertainty.push('Pattern could match other middleware or libraries');
        }

        if (factors.contextQuality < 0.6) {
            uncertainty.push('Surrounding code context is unclear or ambiguous');
        }

        if (factors.consistency < 0.8) {
            uncertainty.push('Inconsistent patterns found across different files');
        }

        const partialMatches = evidence.filter(e => e.matchQuality === 'partial').length;
        if (partialMatches > evidence.length * 0.3) {
            uncertainty.push('Many matches are partial or inferred rather than exact');
        }

        return uncertainty;
    }

    /**
     * Generate human-readable reasoning for confidence score
     */
    private generateReasoning(
        patternType: 'initialize' | 'authenticate' | 'strategy' | 'route-protection',
        factors: ConfidenceFactors,
        evidence: EvidenceItem[]
    ): string {
        const uniqueFiles = new Set(evidence.map(e => e.file)).size;
        const exactMatches = evidence.filter(e => e.matchQuality === 'exact').length;
        
        const patternNames = {
            'initialize': 'passport.initialize()',
            'authenticate': 'passport.authenticate()',
            'strategy': 'passport.use() strategy configuration',
            'route-protection': 'route-level authentication'
        };

        let reasoning = `Detected ${patternNames[patternType]} pattern with `;
        reasoning += `${exactMatches}/${evidence.length} exact matches across ${uniqueFiles} file(s). `;
        
        if (factors.patternComplexity > 0.8) {
            reasoning += 'Pattern is highly specific to Passport.js. ';
        } else if (factors.patternComplexity > 0.6) {
            reasoning += 'Pattern is moderately specific but could match other middleware. ';
        } else {
            reasoning += 'Pattern is generic and needs additional context validation. ';
        }

        if (factors.contextQuality > 0.8) {
            reasoning += 'Strong contextual evidence (Express app setup, Passport imports).';
        } else if (factors.contextQuality > 0.6) {
            reasoning += 'Moderate contextual evidence.';
        } else {
            reasoning += 'Limited contextual evidence - verification recommended.';
        }

        return reasoning;
    }

    /**
     * Create a low confidence result for edge cases
     */
    private createLowConfidenceResult(reason: string, evidence: EvidenceItem[]): ConfidenceResult {
        return {
            score: 0,
            level: 'low',
            factors: {
                evidenceCount: 0,
                patternComplexity: 0,
                contextQuality: 0,
                consistency: 0,
                recency: 0
            },
            evidence,
            reasoning: reason,
            uncertainty: ['No evidence found for this pattern']
        };
    }
}

// Example usage and testing
export function testConfidenceScoring() {
    const scorer = new PassportConfidenceScorer();

    // Test case 1: Strong evidence for passport.initialize()
    const strongEvidence: EvidenceItem[] = [
        {
            file: 'src/app.ts',
            line: 124,
            pattern: 'this.app.use(passport.initialize())',
            matchQuality: 'exact',
            context: 'this.app.use(passport.initialize()); // Initialize passport'
        }
    ];

    console.log('🧪 Testing Confidence Scoring...\n');

    const result1 = scorer.calculateConfidence('initialize', strongEvidence);
    console.log('Test 1 - Strong Evidence for passport.initialize():');
    console.log(`  Score: ${result1.score}% (${result1.level})`);
    console.log(`  Reasoning: ${result1.reasoning}`);
    if (result1.uncertainty.length > 0) {
        console.log(`  Uncertainty: ${result1.uncertainty.join(', ')}`);
    }
    console.log('');

    // Test case 2: Weak evidence (partial matches)
    const weakEvidence: EvidenceItem[] = [
        {
            file: 'src/unknown.js',
            line: 50,
            pattern: 'passport.something()',
            matchQuality: 'partial',
            context: 'const result = passport.something();'
        }
    ];

    const result2 = scorer.calculateConfidence('authenticate', weakEvidence);
    console.log('Test 2 - Weak Evidence:');
    console.log(`  Score: ${result2.score}% (${result2.level})`);
    console.log(`  Reasoning: ${result2.reasoning}`);
    if (result2.uncertainty.length > 0) {
        console.log(`  Uncertainty: ${result2.uncertainty.join(', ')}`);
    }
    console.log('');

    // Test case 3: No evidence
    const result3 = scorer.calculateConfidence('strategy', []);
    console.log('Test 3 - No Evidence:');
    console.log(`  Score: ${result3.score}% (${result3.level})`);
    console.log(`  Reasoning: ${result3.reasoning}`);
    console.log('');

    return { strongResult: result1, weakResult: result2, noEvidenceResult: result3 };
}