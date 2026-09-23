import { AccuracyMeasurementFramework } from './dist/accuracy-measurement-framework.js';

// Test pattern matching logic
function debugPatternMatching() {
    console.log('🔍 Debugging Pattern Matching Logic...\n');
    
    const framework = new AccuracyMeasurementFramework();
    
    // Test cases from ground truth vs detected
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
            name: 'Initialize Pattern',
            expected: {
                expectedPattern: "passport.initialize()",
                patternType: 'initialize'
            },
            detected: {
                pattern: "app.use(passport.initialize())",
                file: 'app.ts',
                line: 124
            }
        }
    ];
    
    for (const testCase of testCases) {
        console.log(`📝 Testing: ${testCase.name}`);
        console.log(`  Expected: "${testCase.expected.expectedPattern}"`);
        console.log(`  Detected: "${testCase.detected.pattern}"`);
        
        // Test the pattern matching logic directly
        const normalizedExpected = testCase.expected.expectedPattern.toLowerCase().replace(/\\s+/g, ' ');
        const normalizedDetected = testCase.detected.pattern.toLowerCase().replace(/\\s+/g, ' ');
        
        const matches1 = normalizedDetected.includes(normalizedExpected);
        const matches2 = normalizedExpected.includes(normalizedDetected);
        const matches = matches1 || matches2;
        
        console.log(`  Normalized Expected: "${normalizedExpected}"`);
        console.log(`  Normalized Detected: "${normalizedDetected}"`);
        console.log(`  Detected includes Expected: ${matches1}`);
        console.log(`  Expected includes Detected: ${matches2}`);
        console.log(`  Final Match: ${matches ? '✅' : '❌'}`);
        console.log('');
    }
}

debugPatternMatching();