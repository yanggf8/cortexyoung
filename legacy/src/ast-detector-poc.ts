/**
 * AST-based Passport Pattern Detector - Proof of Concept
 * 
 * This POC demonstrates how to use @typescript-eslint/parser to detect:
 * 1. app.use(passport.initialize())
 * 2. app.use(passport.authenticate(...))
 * 3. passport.use(new Strategy(...))
 * 4. passport.authenticate('strategy', options)
 */

import { parse } from '@typescript-eslint/typescript-estree';
import { AST_NODE_TYPES, TSESTree } from '@typescript-eslint/typescript-estree';
import fs from 'fs';
import path from 'path';

export interface PassportPattern {
    type: 'initialize' | 'authenticate' | 'strategy' | 'route-protection';
    pattern: string;
    file: string;
    line: number;
    confidence: number;
    evidence: string;
}

export class ASTPassportDetector {
    private patterns: PassportPattern[] = [];

    /**
     * Parse a JavaScript/TypeScript file and detect Passport patterns
     */
    public async detectPatternsInFile(filePath: string): Promise<PassportPattern[]> {
        const fileContent = fs.readFileSync(filePath, 'utf-8');
        const fileName = path.basename(filePath);
        
        try {
            const ast = parse(fileContent, {
                loc: true,
                range: true,
                tokens: false,
                comments: false,
                errorOnUnknownASTType: false,
                errorOnTypeScriptSyntacticAndSemanticIssues: false,
                jsx: false
            });

            this.patterns = [];
            this.traverseAST(ast, fileName);
            
            return this.patterns;
        } catch (error) {
            console.warn(`Failed to parse ${filePath}: ${error}`);
            return [];
        }
    }

    /**
     * Traverse the AST and detect Passport patterns
     */
    private traverseAST(node: TSESTree.Node, fileName: string): void {
        // Handle CallExpressions (function calls)
        if (node.type === AST_NODE_TYPES.CallExpression) {
            this.handleCallExpression(node, fileName);
        }

        // Recursively traverse child nodes
        for (const key in node) {
            const child = (node as any)[key];
            if (child && typeof child === 'object') {
                if (Array.isArray(child)) {
                    child.forEach(item => {
                        if (item && typeof item === 'object' && item.type) {
                            this.traverseAST(item, fileName);
                        }
                    });
                } else if (child.type) {
                    this.traverseAST(child, fileName);
                }
            }
        }
    }

    /**
     * Handle CallExpression nodes to detect Passport patterns
     */
    private handleCallExpression(node: TSESTree.CallExpression, fileName: string): void {
        // Pattern 1: app.use(passport.initialize())
        if (this.isAppUsePassportInitialize(node)) {
            this.addPattern({
                type: 'initialize',
                pattern: 'app.use(passport.initialize())',
                file: fileName,
                line: node.loc?.start.line || 0,
                confidence: 100,
                evidence: this.getNodeText(node)
            });
        }

        // Pattern 2: app.use(passport.authenticate(...))
        if (this.isAppUsePassportAuthenticate(node)) {
            this.addPattern({
                type: 'authenticate',
                pattern: 'app.use(passport.authenticate(...))',
                file: fileName,
                line: node.loc?.start.line || 0,
                confidence: 100,
                evidence: this.getNodeText(node)
            });
        }

        // Pattern 3: passport.use(new Strategy(...))
        if (this.isPassportUseStrategy(node)) {
            const strategyName = this.extractStrategyName(node);
            this.addPattern({
                type: 'strategy',
                pattern: `passport.use(new ${strategyName}(...))`,
                file: fileName,
                line: node.loc?.start.line || 0,
                confidence: 95,
                evidence: this.getNodeText(node)
            });
        }

        // Pattern 4: passport.authenticate('strategy', options)
        if (this.isPassportAuthenticate(node)) {
            const strategy = this.extractAuthStrategy(node);
            this.addPattern({
                type: 'authenticate',
                pattern: `passport.authenticate('${strategy}', ...)`,
                file: fileName,
                line: node.loc?.start.line || 0,
                confidence: 95,
                evidence: this.getNodeText(node)
            });
        }
    }

    /**
     * Check if node matches: app.use(passport.initialize()) or this.app.use(passport.initialize())
     */
    private isAppUsePassportInitialize(node: TSESTree.CallExpression): boolean {
        // Check if this is app.use(...) or this.app.use(...)
        if (!this.isAppUseCall(node)) return false;
        
        // Check if the first argument is passport.initialize()
        const firstArg = node.arguments[0];
        if (!firstArg || firstArg.type !== AST_NODE_TYPES.CallExpression) return false;
        
        return this.isMethodCall(firstArg, 'passport', 'initialize');
    }

    /**
     * Check if node matches: app.use(passport.authenticate(...)) or this.app.use(passport.authenticate(...))
     */
    private isAppUsePassportAuthenticate(node: TSESTree.CallExpression): boolean {
        // Check if this is app.use(...) or this.app.use(...)
        if (!this.isAppUseCall(node)) return false;
        
        // Check if the first argument is passport.authenticate(...)
        const firstArg = node.arguments[0];
        if (!firstArg || firstArg.type !== AST_NODE_TYPES.CallExpression) return false;
        
        return this.isMethodCall(firstArg, 'passport', 'authenticate');
    }

    /**
     * Helper: Check if node is app.use(...) or this.app.use(...)
     */
    private isAppUseCall(node: TSESTree.CallExpression): boolean {
        if (node.callee.type !== AST_NODE_TYPES.MemberExpression) return false;
        
        const callee = node.callee;
        
        // Check method name is 'use'
        if (callee.property.type !== AST_NODE_TYPES.Identifier) return false;
        if (callee.property.name !== 'use') return false;
        
        // Check if it's app.use or this.app.use
        if (callee.object.type === AST_NODE_TYPES.Identifier && callee.object.name === 'app') {
            return true; // app.use
        }
        
        if (callee.object.type === AST_NODE_TYPES.MemberExpression) {
            // Check for this.app.use
            if (callee.object.object.type === AST_NODE_TYPES.ThisExpression &&
                callee.object.property.type === AST_NODE_TYPES.Identifier &&
                callee.object.property.name === 'app') {
                return true; // this.app.use
            }
        }
        
        return false;
    }

    /**
     * Check if node matches: passport.use(new Strategy(...))
     */
    private isPassportUseStrategy(node: TSESTree.CallExpression): boolean {
        // Check if this is passport.use(...)
        if (!this.isMethodCall(node, 'passport', 'use')) return false;
        
        // Check if the first argument is new Strategy(...)
        const firstArg = node.arguments[0];
        if (!firstArg || firstArg.type !== AST_NODE_TYPES.NewExpression) return false;
        
        // Check if it's a Strategy constructor (ends with 'Strategy')
        if (firstArg.callee.type === AST_NODE_TYPES.Identifier) {
            return firstArg.callee.name.endsWith('Strategy');
        }
        
        return false;
    }

    /**
     * Check if node matches: passport.authenticate('strategy', options)
     */
    private isPassportAuthenticate(node: TSESTree.CallExpression): boolean {
        return this.isMethodCall(node, 'passport', 'authenticate');
    }

    /**
     * Helper: Check if node is a method call (object.method())
     */
    private isMethodCall(node: TSESTree.CallExpression, objectName: string, methodName: string): boolean {
        if (node.callee.type !== AST_NODE_TYPES.MemberExpression) return false;
        
        const callee = node.callee;
        
        // Check object name
        if (callee.object.type !== AST_NODE_TYPES.Identifier) return false;
        if (callee.object.name !== objectName) return false;
        
        // Check method name
        if (callee.property.type !== AST_NODE_TYPES.Identifier) return false;
        if (callee.property.name !== methodName) return false;
        
        return true;
    }

    /**
     * Extract strategy name from passport.use(new Strategy(...))
     */
    private extractStrategyName(node: TSESTree.CallExpression): string {
        const firstArg = node.arguments[0];
        if (firstArg?.type === AST_NODE_TYPES.NewExpression && 
            firstArg.callee.type === AST_NODE_TYPES.Identifier) {
            return firstArg.callee.name;
        }
        return 'Unknown';
    }

    /**
     * Extract strategy name from passport.authenticate('strategy', ...)
     */
    private extractAuthStrategy(node: TSESTree.CallExpression): string {
        const firstArg = node.arguments[0];
        if (firstArg?.type === AST_NODE_TYPES.Literal && typeof firstArg.value === 'string') {
            return firstArg.value;
        }
        return 'unknown';
    }

    /**
     * Get text representation of AST node (simplified)
     */
    private getNodeText(node: TSESTree.Node): string {
        // This is a simplified version - in practice you'd want to reconstruct the original text
        if (node.type === AST_NODE_TYPES.CallExpression) {
            return `${this.getNodeText(node.callee)}(...)`;
        }
        if (node.type === AST_NODE_TYPES.MemberExpression) {
            return `${this.getNodeText(node.object)}.${this.getNodeText(node.property)}`;
        }
        if (node.type === AST_NODE_TYPES.Identifier) {
            return node.name;
        }
        return 'unknown';
    }

    /**
     * Add a detected pattern to the results
     */
    private addPattern(pattern: PassportPattern): void {
        this.patterns.push(pattern);
    }

    /**
     * Get all detected patterns
     */
    public getPatterns(): PassportPattern[] {
        return this.patterns;
    }

    /**
     * Get summary statistics
     */
    public getSummary() {
        const byType = this.patterns.reduce((acc, p) => {
            acc[p.type] = (acc[p.type] || 0) + 1;
            return acc;
        }, {} as Record<string, number>);

        return {
            total: this.patterns.length,
            byType,
            avgConfidence: this.patterns.reduce((sum, p) => sum + p.confidence, 0) / this.patterns.length || 0
        };
    }
}

// Example usage and testing
async function testASTDetector() {
    const detector = new ASTPassportDetector();
    
    // Test with our ground truth projects
    const testFiles = [
        '/tmp/ground-truth-testing/express-jwt-passport-local-mongoose-winston/src/app.ts',
        '/tmp/ground-truth-testing/express-jwt-passport-local-mongoose-winston/src/passport/passport.ts'
    ];

    console.log('🔍 Testing AST Passport Pattern Detection...\n');

    for (const file of testFiles) {
        if (fs.existsSync(file)) {
            console.log(`📁 Analyzing: ${path.basename(file)}`);
            const patterns = await detector.detectPatternsInFile(file);
            
            if (patterns.length > 0) {
                patterns.forEach(p => {
                    console.log(`  ✅ Found: ${p.pattern} (Line ${p.line}, ${p.confidence}% confidence)`);
                    console.log(`     Evidence: ${p.evidence}`);
                });
            } else {
                console.log('  ❌ No patterns detected');
            }
            console.log('');
        }
    }

    const summary = detector.getSummary();
    console.log('📊 Summary:', summary);
}

// Export for testing
export { testASTDetector };