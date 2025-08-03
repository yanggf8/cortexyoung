import Parser from 'tree-sitter';
import TypeScript from 'tree-sitter-typescript';
import JavaScript from 'tree-sitter-javascript';
import {
  CodeSymbol,
  CodeRelationship,
  RelationshipType,
  FileAnalysisResult,
  ImportDeclaration,
  ExportDeclaration,
  AnalysisError,
  RelationshipMetadata
} from './relationship-types';

export class CallGraphAnalyzer {
  private parser: Parser;
  private tsParser: Parser;

  constructor() {
    // Initialize parsers for JavaScript and TypeScript
    this.parser = new Parser();
    this.parser.setLanguage(JavaScript);
    
    this.tsParser = new Parser();
    this.tsParser.setLanguage(TypeScript.typescript);
  }

  async analyzeFile(filePath: string, content: string): Promise<FileAnalysisResult> {
    const isTypeScript = filePath.endsWith('.ts') || filePath.endsWith('.tsx');
    const parser = isTypeScript ? this.tsParser : this.parser;
    
    try {
      const tree = parser.parse(content);
      const symbols: CodeSymbol[] = [];
      const relationships: CodeRelationship[] = [];
      const imports: ImportDeclaration[] = [];
      const exports: ExportDeclaration[] = [];
      const errors: AnalysisError[] = [];

      // Extract symbols and relationships
      this.traverseNode(tree.rootNode, content, filePath, symbols, relationships, imports, exports, errors);

      return {
        symbols,
        relationships,
        imports,
        exports,
        errors
      };
    } catch (error) {
      return {
        symbols: [],
        relationships: [],
        imports: [],
        exports: [],
        errors: [{
          type: 'parse_error',
          message: `Failed to parse ${filePath}: ${error instanceof Error ? error.message : 'Unknown error'}`,
          severity: 'error'
        }]
      };
    }
  }

  private traverseNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    symbols: CodeSymbol[],
    relationships: CodeRelationship[],
    imports: ImportDeclaration[],
    exports: ExportDeclaration[],
    errors: AnalysisError[]
  ): void {
    // Process different node types
    switch (node.type) {
      case 'function_declaration':
      case 'method_definition':
      case 'arrow_function':
      case 'function_expression':
        this.processFunctionNode(node, content, filePath, symbols, relationships);
        break;
        
      case 'class_declaration':
        this.processClassNode(node, content, filePath, symbols, relationships);
        break;
        
      case 'call_expression':
        this.processCallExpression(node, content, filePath, relationships);
        break;
        
      case 'import_statement':
      case 'import_declaration':
        this.processImportNode(node, content, filePath, imports, relationships);
        break;
        
      case 'export_statement':
      case 'export_declaration':
        this.processExportNode(node, content, filePath, exports);
        break;
        
      case 'variable_declaration':
      case 'lexical_declaration':
        this.processVariableNode(node, content, filePath, symbols, relationships);
        break;
        
      case 'assignment_expression':
        this.processAssignmentNode(node, content, filePath, relationships);
        break;
        
      case 'try_statement':
        this.processTryStatement(node, content, filePath, relationships);
        break;
    }

    // Recursively process child nodes
    for (let i = 0; i < node.childCount; i++) {
      this.traverseNode(node.child(i)!, content, filePath, symbols, relationships, imports, exports, errors);
    }
  }

  private processFunctionNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    symbols: CodeSymbol[],
    relationships: CodeRelationship[]
  ): void {
    const nameNode = this.findChildByType(node, 'identifier');
    if (!nameNode) return;

    const functionName = this.getNodeText(nameNode, content);
    const startLine = node.startPosition.row + 1;
    const endLine = node.endPosition.row + 1;

    // Extract function signature
    const signature = this.extractFunctionSignature(node, content);
    
    // Determine scope
    const scope = this.determineScope(node);
    
    const symbol: CodeSymbol = {
      id: `${filePath}:${functionName}:${startLine}`,
      name: functionName,
      type: node.type === 'method_definition' ? 'method' : 'function',
      filePath,
      startLine,
      endLine,
      signature,
      scope
    };

    symbols.push(symbol);

    // Extract parameters for data flow analysis
    this.extractParameterRelationships(node, content, filePath, symbol.id, relationships);
  }

  private processClassNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    symbols: CodeSymbol[],
    relationships: CodeRelationship[]
  ): void {
    const nameNode = this.findChildByType(node, 'identifier');
    if (!nameNode) return;

    const className = this.getNodeText(nameNode, content);
    const startLine = node.startPosition.row + 1;
    const endLine = node.endPosition.row + 1;

    const symbol: CodeSymbol = {
      id: `${filePath}:${className}:${startLine}`,
      name: className,
      type: 'class',
      filePath,
      startLine,
      endLine,
      scope: 'global'
    };

    symbols.push(symbol);

    // Process inheritance relationships
    const heritageClause = this.findChildByType(node, 'class_heritage');
    if (heritageClause) {
      this.processInheritanceRelationships(heritageClause, content, filePath, symbol.id, relationships);
    }
  }

  private processCallExpression(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    relationships: CodeRelationship[]
  ): void {
    const functionNode = node.child(0);
    if (!functionNode) return;

    const callerContext = this.findContainingFunction(node);
    if (!callerContext) return;

    const callerName = this.getNodeText(this.findChildByType(callerContext, 'identifier')!, content);
    const callerLine = callerContext.startPosition.row + 1;
    const callerId = `${filePath}:${callerName}:${callerLine}`;

    // Extract called function information
    const calledFunction = this.extractCalledFunction(functionNode, content);
    if (!calledFunction) return;

    // Determine call type
    const callType = this.determineCallType(node, content);
    
    // Extract parameters for data flow
    const parameters = this.extractCallParameters(node, content);

    const relationship: CodeRelationship = {
      id: `${callerId}:calls:${calledFunction.name}:${node.startPosition.row + 1}`,
      fromSymbol: callerId,
      toSymbol: calledFunction.target || `${filePath}:${calledFunction.name}:unknown`,
      type: 'calls',
      strength: this.calculateCallStrength(node, content),
      confidence: calledFunction.confidence,
      metadata: {
        callType,
        parameters,
        sourceLocation: {
          line: node.startPosition.row + 1,
          column: node.startPosition.column
        }
      }
    };

    relationships.push(relationship);
  }

  private processImportNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    imports: ImportDeclaration[],
    relationships: CodeRelationship[]
  ): void {
    const sourceNode = this.findChildByType(node, 'string');
    if (!sourceNode) return;

    const source = this.getNodeText(sourceNode, content).slice(1, -1); // Remove quotes
    const line = node.startPosition.row + 1;

    // Extract imported symbols
    const importedSymbols = this.extractImportedSymbols(node, content);
    const importType = this.determineImportType(node, content);

    const importDecl: ImportDeclaration = {
      source,
      importedSymbols,
      importType,
      line,
      isTypeOnly: this.isTypeOnlyImport(node, content)
    };

    imports.push(importDecl);

    // Create import relationships
    importedSymbols.forEach(symbolName => {
      const relationship: CodeRelationship = {
        id: `${filePath}:imports:${source}:${symbolName}:${line}`,
        fromSymbol: `${filePath}:module:1`,
        toSymbol: `${source}:${symbolName}:unknown`,
        type: 'imports',
        strength: 1.0,
        confidence: 0.9,
        metadata: {
          importType,
          importedSymbols: [symbolName],
          sourceLocation: { line, column: node.startPosition.column }
        }
      };

      relationships.push(relationship);
    });
  }

  private processExportNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    exports: ExportDeclaration[]
  ): void {
    const exportedSymbol = this.extractExportedSymbol(node, content);
    if (!exportedSymbol) return;

    const exportType = this.determineExportType(node, content);
    const line = node.startPosition.row + 1;

    const exportDecl: ExportDeclaration = {
      exportedSymbol,
      exportType,
      line,
      isReExport: this.isReExport(node, content),
      originalSource: this.getReExportSource(node, content)
    };

    exports.push(exportDecl);
  }

  private processVariableNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    symbols: CodeSymbol[],
    relationships: CodeRelationship[]
  ): void {
    const declarators = this.findChildrenByType(node, 'variable_declarator');
    
    declarators.forEach(declarator => {
      const nameNode = this.findChildByType(declarator, 'identifier');
      if (!nameNode) return;

      const variableName = this.getNodeText(nameNode, content);
      const line = declarator.startPosition.row + 1;

      const symbol: CodeSymbol = {
        id: `${filePath}:${variableName}:${line}`,
        name: variableName,
        type: 'variable',
        filePath,
        startLine: line,
        endLine: line,
        scope: this.determineScope(declarator)
      };

      symbols.push(symbol);

      // Process assignment relationships
      const initNode = this.findChildByType(declarator, 'assignment_expression');
      if (initNode) {
        this.processDataFlowFromAssignment(initNode, content, filePath, symbol.id, relationships);
      }
    });
  }

  private processAssignmentNode(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    relationships: CodeRelationship[]
  ): void {
    const leftNode = node.child(0);
    const rightNode = node.child(2); // Assignment operator is child(1)
    
    if (!leftNode || !rightNode) return;

    const targetVariable = this.extractVariableReference(leftNode, content);
    const sourceValue = this.extractValueSource(rightNode, content);

    if (targetVariable && sourceValue) {
      const relationship: CodeRelationship = {
        id: `${targetVariable}:assigns:${sourceValue.source}:${node.startPosition.row + 1}`,
        fromSymbol: sourceValue.source,
        toSymbol: targetVariable,
        type: 'assigns',
        strength: 0.8,
        confidence: sourceValue.confidence,
        metadata: {
          flowType: 'assignment',
          sourceLocation: {
            line: node.startPosition.row + 1,
            column: node.startPosition.column
          }
        }
      };

      relationships.push(relationship);
    }
  }

  private processTryStatement(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    relationships: CodeRelationship[]
  ): void {
    const tryBlock = this.findChildByType(node, 'statement_block');
    const catchClause = this.findChildByType(node, 'catch_clause');
    
    if (!tryBlock || !catchClause) return;

    const containingFunction = this.findContainingFunction(node);
    if (!containingFunction) return;

    const functionName = this.getNodeText(this.findChildByType(containingFunction, 'identifier')!, content);
    const functionId = `${filePath}:${functionName}:${containingFunction.startPosition.row + 1}`;

    // Extract error type from catch parameter
    const errorType = this.extractCatchErrorType(catchClause, content);

    const relationship: CodeRelationship = {
      id: `${functionId}:catches:${errorType}:${catchClause.startPosition.row + 1}`,
      fromSymbol: functionId,
      toSymbol: `${filePath}:${errorType}:error`,
      type: 'catches',
      strength: 0.7,
      confidence: 0.8,
      metadata: {
        errorType,
        handlingStrategy: 'try_catch',
        sourceLocation: {
          line: catchClause.startPosition.row + 1,
          column: catchClause.startPosition.column
        }
      }
    };

    relationships.push(relationship);
  }

  // Helper methods
  private findChildByType(node: Parser.SyntaxNode, type: string): Parser.SyntaxNode | null {
    for (let i = 0; i < node.childCount; i++) {
      const child = node.child(i)!;
      if (child.type === type) return child;
    }
    return null;
  }

  private findChildrenByType(node: Parser.SyntaxNode, type: string): Parser.SyntaxNode[] {
    const children: Parser.SyntaxNode[] = [];
    for (let i = 0; i < node.childCount; i++) {
      const child = node.child(i)!;
      if (child.type === type) children.push(child);
    }
    return children;
  }

  private getNodeText(node: Parser.SyntaxNode, content: string): string {
    return content.slice(node.startIndex, node.endIndex);
  }

  private findContainingFunction(node: Parser.SyntaxNode): Parser.SyntaxNode | null {
    let current = node.parent;
    while (current) {
      if (['function_declaration', 'method_definition', 'arrow_function', 'function_expression'].includes(current.type)) {
        return current;
      }
      current = current.parent;
    }
    return null;
  }

  private extractFunctionSignature(node: Parser.SyntaxNode, content: string): string {
    const params = this.findChildByType(node, 'formal_parameters');
    const returnType = this.findChildByType(node, 'type_annotation');
    
    let signature = '';
    if (params) {
      signature += this.getNodeText(params, content);
    }
    if (returnType) {
      signature += ': ' + this.getNodeText(returnType, content);
    }
    
    return signature;
  }

  private determineScope(node: Parser.SyntaxNode): 'global' | 'local' | 'class' | 'module' {
    let current = node.parent;
    while (current) {
      if (current.type === 'class_declaration') return 'class';
      if (['function_declaration', 'method_definition', 'arrow_function'].includes(current.type)) return 'local';
      current = current.parent;
    }
    return 'global';
  }

  private extractCalledFunction(node: Parser.SyntaxNode, content: string): { name: string; target?: string; confidence: number } | null {
    if (node.type === 'identifier') {
      return {
        name: this.getNodeText(node, content),
        confidence: 0.9
      };
    }
    
    if (node.type === 'member_expression') {
      const property = this.findChildByType(node, 'property_identifier');
      if (property) {
        return {
          name: this.getNodeText(property, content),
          confidence: 0.8
        };
      }
    }
    
    return null;
  }

  private determineCallType(node: Parser.SyntaxNode, content: string): 'direct' | 'indirect' | 'async' | 'callback' {
    // Check for await keyword
    const parent = node.parent;
    if (parent && parent.type === 'await_expression') {
      return 'async';
    }
    
    // Check for callback pattern (function passed as argument)
    const args = this.findChildByType(node, 'arguments');
    if (args) {
      const functionArgs = this.findChildrenByType(args, 'arrow_function').concat(
        this.findChildrenByType(args, 'function_expression')
      );
      if (functionArgs.length > 0) {
        return 'callback';
      }
    }
    
    // Check for method call vs direct function call
    const callee = node.child(0);
    if (callee && callee.type === 'member_expression') {
      return 'indirect';
    }
    
    return 'direct';
  }

  private extractCallParameters(node: Parser.SyntaxNode, content: string): string[] {
    const args = this.findChildByType(node, 'arguments');
    if (!args) return [];
    
    const parameters: string[] = [];
    for (let i = 0; i < args.childCount; i++) {
      const arg = args.child(i)!;
      if (arg.type !== ',' && arg.type !== '(' && arg.type !== ')') {
        parameters.push(this.getNodeText(arg, content));
      }
    }
    
    return parameters;
  }

  private calculateCallStrength(node: Parser.SyntaxNode, content: string): number {
    // Base strength
    let strength = 0.7;
    
    // Increase strength for direct calls
    const callee = node.child(0);
    if (callee && callee.type === 'identifier') {
      strength += 0.2;
    }
    
    // Increase strength for calls with arguments
    const args = this.findChildByType(node, 'arguments');
    if (args && args.childCount > 2) { // More than just parentheses
      strength += 0.1;
    }
    
    return Math.min(strength, 1.0);
  }

  private extractImportedSymbols(node: Parser.SyntaxNode, content: string): string[] {
    const symbols: string[] = [];
    
    // Handle different import patterns
    const importClause = this.findChildByType(node, 'import_clause');
    if (importClause) {
      // Named imports: import { a, b } from 'module'
      const namedImports = this.findChildByType(importClause, 'named_imports');
      if (namedImports) {
        const specs = this.findChildrenByType(namedImports, 'import_specifier');
        specs.forEach(spec => {
          const name = this.findChildByType(spec, 'identifier');
          if (name) symbols.push(this.getNodeText(name, content));
        });
      }
      
      // Default import: import defaultName from 'module'
      const defaultImport = this.findChildByType(importClause, 'identifier');
      if (defaultImport) {
        symbols.push(this.getNodeText(defaultImport, content));
      }
      
      // Namespace import: import * as name from 'module'
      const namespaceImport = this.findChildByType(importClause, 'namespace_import');
      if (namespaceImport) {
        const name = this.findChildByType(namespaceImport, 'identifier');
        if (name) symbols.push(this.getNodeText(name, content));
      }
    }
    
    return symbols;
  }

  private determineImportType(node: Parser.SyntaxNode, content: string): 'default' | 'named' | 'namespace' | 'dynamic' {
    const importClause = this.findChildByType(node, 'import_clause');
    if (!importClause) return 'dynamic';
    
    if (this.findChildByType(importClause, 'namespace_import')) return 'namespace';
    if (this.findChildByType(importClause, 'named_imports')) return 'named';
    return 'default';
  }

  private isTypeOnlyImport(node: Parser.SyntaxNode, content: string): boolean {
    const text = this.getNodeText(node, content);
    return text.includes('import type') || text.includes('import { type');
  }

  private extractExportedSymbol(node: Parser.SyntaxNode, content: string): string | null {
    // Handle different export patterns
    const declaration = this.findChildByType(node, 'function_declaration') ||
                       this.findChildByType(node, 'class_declaration') ||
                       this.findChildByType(node, 'variable_declaration');
    
    if (declaration) {
      const name = this.findChildByType(declaration, 'identifier');
      if (name) return this.getNodeText(name, content);
    }
    
    return null;
  }

  private determineExportType(node: Parser.SyntaxNode, content: string): 'default' | 'named' | 'namespace' {
    const text = this.getNodeText(node, content);
    if (text.includes('export default')) return 'default';
    if (text.includes('export *')) return 'namespace';
    return 'named';
  }

  private isReExport(node: Parser.SyntaxNode, content: string): boolean {
    return this.getNodeText(node, content).includes('from');
  }

  private getReExportSource(node: Parser.SyntaxNode, content: string): string | undefined {
    if (!this.isReExport(node, content)) return undefined;
    
    const source = this.findChildByType(node, 'string');
    return source ? this.getNodeText(source, content).slice(1, -1) : undefined;
  }

  private extractVariableReference(node: Parser.SyntaxNode, content: string): string | null {
    if (node.type === 'identifier') {
      return this.getNodeText(node, content);
    }
    
    if (node.type === 'member_expression') {
      const object = node.child(0);
      const property = node.child(2);
      if (object && property) {
        return `${this.getNodeText(object, content)}.${this.getNodeText(property, content)}`;
      }
    }
    
    return null;
  }

  private extractValueSource(node: Parser.SyntaxNode, content: string): { source: string; confidence: number } | null {
    if (node.type === 'identifier') {
      return {
        source: this.getNodeText(node, content),
        confidence: 0.9
      };
    }
    
    if (node.type === 'call_expression') {
      const callee = node.child(0);
      if (callee) {
        return {
          source: this.getNodeText(callee, content),
          confidence: 0.8
        };
      }
    }
    
    return null;
  }

  private extractCatchErrorType(catchClause: Parser.SyntaxNode, content: string): string {
    const param = this.findChildByType(catchClause, 'identifier');
    return param ? this.getNodeText(param, content) : 'Error';
  }

  private processInheritanceRelationships(
    heritageClause: Parser.SyntaxNode,
    content: string,
    filePath: string,
    classId: string,
    relationships: CodeRelationship[]
  ): void {
    // Implementation for inheritance analysis
    // This would extract extends/implements relationships
  }

  private extractParameterRelationships(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    functionId: string,
    relationships: CodeRelationship[]
  ): void {
    // Implementation for parameter data flow analysis
    // This would track how parameters flow through the function
  }

  private processDataFlowFromAssignment(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string,
    variableId: string,
    relationships: CodeRelationship[]
  ): void {
    // Implementation for data flow from assignments
    // This would track how values flow between variables
  }
}