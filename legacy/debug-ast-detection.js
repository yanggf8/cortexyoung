import { ASTPassportDetector } from './dist/ast-detector-poc.js';
import fs from 'fs';

// Test AST detection on sample patterns
async function debugASTDetection() {
    console.log('🔍 Debugging AST Pattern Detection Issues...\n');
    
    // Create test files with expected patterns
    const testPatterns = [
        {
            name: 'passport-initialize',
            code: `
const express = require('express');
const passport = require('passport');
const app = express();

app.use(passport.initialize());
app.use(passport.session());
`
        },
        {
            name: 'passport-authenticate',
            code: `
const passport = require('passport');
const express = require('express');
const router = express.Router();

router.get('/protected', passport.authenticate('jwt', { session: false }), (req, res) => {
    res.json({ message: 'Protected route' });
});

app.use('/auth', passport.authenticate('local'), authController.login);
`
        },
        {
            name: 'passport-strategy',
            code: `
const passport = require('passport');
const LocalStrategy = require('passport-local').Strategy;
const JWTStrategy = require('passport-jwt').Strategy;

passport.use(new LocalStrategy({
    usernameField: 'email',
    passwordField: 'password'
}, async (email, password, done) => {
    // authentication logic
}));

passport.use(new JWTStrategy({
    jwtFromRequest: ExtractJwt.fromAuthHeaderAsBearerToken(),
    secretOrKey: process.env.JWT_SECRET
}, async (payload, done) => {
    // JWT validation logic
}));
`
        }
    ];
    
    const detector = new ASTPassportDetector();
    
    for (const testPattern of testPatterns) {
        console.log(`📝 Testing: ${testPattern.name}`);
        
        // Write test file
        const testFile = `/tmp/${testPattern.name}.js`;
        fs.writeFileSync(testFile, testPattern.code);
        
        try {
            const patterns = await detector.detectPatternsInFile(testFile);
            
            if (patterns.length > 0) {
                console.log(`  ✅ Found ${patterns.length} pattern(s):`);
                patterns.forEach(p => {
                    console.log(`    - ${p.type}: ${p.pattern} (${p.confidence}% confidence)`);
                    console.log(`      Evidence: ${p.evidence}`);
                });
            } else {
                console.log('  ❌ No patterns detected');
            }
        } catch (error) {
            console.log(`  ❌ Error: ${error.message}`);
        }
        
        // Clean up
        fs.unlinkSync(testFile);
        console.log('');
    }
    
    const summary = detector.getSummary();
    console.log('📊 Detection Summary:');
    console.log(`  Total patterns: ${summary.total}`);
    console.log(`  By type:`, summary.byType);
    console.log(`  Average confidence: ${Math.round(summary.avgConfidence)}%`);
}

debugASTDetection();