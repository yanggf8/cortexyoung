// Test the improved pattern matching logic directly
function debugImprovedMatching() {
    console.log('🔍 Testing Improved Pattern Matching Logic...\n');
    
    const testCases = [
        {
            name: 'JWT Authentication',
            expected: {
                expectedPattern: "passport.authenticate('jwt')",
                patternType: 'authenticate'
            },
            detected: {
                pattern: "passport.authenticate('jwt', ...)",
                file: 'app.ts',
                line: 133
            }
        },
        {
            name: 'Local Strategy',
            expected: {
                expectedPattern: "passport.use(new LocalStrategy",
                patternType: 'strategy'
            },
            detected: {
                pattern: "passport.use(new Strategy(...))",
                file: 'passport.ts',
                line: 16
            }
        },
        {
            name: 'JWT Strategy',
            expected: {
                expectedPattern: "passport.use(new JWTStrategy",
                patternType: 'strategy'
            },
            detected: {
                pattern: "passport.use(new JWTStrategy(...))",
                file: 'passport.ts',
                line: 27
            }
        }
    ];
    
    for (const testCase of testCases) {
        console.log(`📝 Testing: ${testCase.name}`);
        console.log(`  Expected: "${testCase.expected.expectedPattern}"`);
        console.log(`  Detected: "${testCase.detected.pattern}"`);
        
        const matches = improvedPatternsMatch(testCase.expected, testCase.detected);
        console.log(`  Final Match: ${matches ? '✅' : '❌'}`);
        console.log('');
    }
}

// Replicate the improved pattern matching logic
function improvedPatternsMatch(expected, detected) {
    const normalizedExpected = expected.expectedPattern.toLowerCase().replace(/\\s+/g, ' ').trim();
    const normalizedDetected = detected.pattern.toLowerCase().replace(/\\s+/g, ' ').trim();
    
    // Remove common AST artifacts like "..." and "()"
    const cleanExpected = normalizedExpected.replace(/\\(\\.\\.\\.\\)/g, '').replace(/\\.\\.\\./g, '');
    const cleanDetected = normalizedDetected.replace(/\\(\\.\\.\\.\\)/g, '').replace(/\\.\\.\\./g, '');
    
    console.log(`    Clean Expected: "${cleanExpected}"`);
    console.log(`    Clean Detected: "${cleanDetected}"`);
    
    // Basic substring matching
    if (cleanDetected.includes(cleanExpected) || cleanExpected.includes(cleanDetected)) {
        console.log(`    Substring match: ✅`);
        return true;
    }
    
    // Handle strategy name variations (LocalStrategy vs Strategy)
    if (expected.patternType === 'strategy') {
        console.log(`    Testing strategy pattern...`);
        const expectedStrategyMatch = cleanExpected.match(/new (\\w+)/);
        const detectedStrategyMatch = cleanDetected.match(/new (\\w+)/);
        
        if (expectedStrategyMatch && detectedStrategyMatch) {
            const expectedStrategy = expectedStrategyMatch[1];
            const detectedStrategy = detectedStrategyMatch[1];
            
            console.log(`    Expected strategy: "${expectedStrategy}"`);
            console.log(`    Detected strategy: "${detectedStrategy}"`);
            
            // Match LocalStrategy with Strategy, JWTStrategy with Strategy, etc.
            if (expectedStrategy.includes('strategy') && detectedStrategy === 'strategy') {
                console.log(`    Strategy name match: ✅`);
                return true;
            }
            if (detectedStrategy.includes('strategy') && expectedStrategy === 'strategy') {
                console.log(`    Strategy name match: ✅`);
                return true;
            }
        }
    }
    
    // Handle authenticate method variations
    if (expected.patternType === 'authenticate') {
        console.log(`    Testing authenticate pattern...`);
        const expectedAuthMatch = cleanExpected.match(/passport\\.authenticate\\s*\\[['"]([^'"]+)['"]/);
        const detectedAuthMatch = cleanDetected.match(/passport\\.authenticate\\s*\\[['"]([^'"]+)['"]/);
        
        console.log(`    Expected auth match:`, expectedAuthMatch);
        console.log(`    Detected auth match:`, detectedAuthMatch);
        
        if (expectedAuthMatch && detectedAuthMatch) {
            // Match if the authentication strategy is the same
            const match = expectedAuthMatch[1] === detectedAuthMatch[1];
            console.log(`    Auth strategy match: ${match ? '✅' : '❌'}`);
            return match;
        }
    }
    
    console.log(`    No match found: ❌`);
    return false;
}

debugImprovedMatching();