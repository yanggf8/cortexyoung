import * as Parser from 'tree-sitter';
import {
  CodeSymbol,
  CodeRelationship,
  RelationshipType,
  RelationshipMetadata
} from './relationship-types';

export interface DataFlowNode {
  id: string;
  type: 'variable' | 'parameter' | 'return' | 'property' | 'function_call';
  name: string;
  filePath: string;
  line: number;
  column: number;
  scope: string;                 // Function or class scope
  dataType?: string;             // Inferred or annotated type
  value?: string;                // Literal value if available
}

export interface DataFlowEdge {
  id: string;
  from: string;                  // DataFlowNode id
  to: string;                    // DataFlowNode id
  type: DataFlowType;
  transformation?: string;        // How data is transformed
  conditions?: string[];          // Conditions under which flow occurs
  strength: number;              // Confidence in this flow
}

export type DataFlowType = 
  | 'assignment'                 // Direct assignment: a = b
  | 'parameter_passing'          // Function call: func(a)
  | 'return_value'               // Function return: return a
  | 'property_access'            // Object property: obj.prop
  | 'array_access'               // Array element: arr[0]
  | 'destructuring'              // Destructuring: {a, b} = obj
  | 'spread'                     // Spread operator: ...arr
  | 'closure_capture'            // Variable captured in closure
  | 'callback_parameter'         // Parameter in callback function
  | 'promise_resolution'         // Promise resolution
  | 'conditional_flow';          // Flow through if/else/switch

export interface DataFlowPath {
  nodes: DataFlowNode[];
  edges: DataFlowEdge[];
  startVariable: string;
  endVariable: string;
  pathType: 'direct' | 'through_function' | 'through_object' | 'conditional';
  transformations: string[];
  confidence: number;
}

export interface VariableState {
  name: string;
  type?: string;
  value?: string;
  scope: string;
  assignments: number[];         // Line numbers where assigned
  accesses: number[];            // Line numbers where accessed
  flowsTo: Set<string>;         // Variables this flows to
  flowsFrom: Set<string>;       // Variables this flows from
  isParameter: boolean;
  isReturnValue: boolean;
  isCapturedInClosure: boolean;
}

export class DataFlowAnalyzer {
  private dataFlowNodes: Map<string, DataFlowNode> = new Map();
  private dataFlowEdges: Map<string, DataFlowEdge> = new Map();
  private variableStates: Map<string, VariableState> = new Map();
  private scopeStack: string[] = [];
  private currentFunction?: string;

  analyzeDataFlow(
    tree: Parser.Tree,
    content: string,
    filePath: string
  ): { nodes: DataFlowNode[]; edges: DataFlowEdge[]; relationships: CodeRelationship[] } {
    this.dataFlowNodes.clear();
    this.dataFlowEdges.clear();
    this.variableStates.clear();
    this.scopeStack = ['global'];

    // Traverse AST and build data flow graph
    this.traverseForDataFlow(tree.rootNode, content, filePath);

    // Convert to relationship format
    const relationships = this.generateDataFlowRelationships();

    return {
      nodes: Array.from(this.dataFlowNodes.values()),
      edges: Array.from(this.dataFlowEdges.values()),
      relationships
    };
  }

  private traverseForDataFlow(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    switch (node.type) {
      case 'function_declaration':
      case 'method_definition':
      case 'arrow_function':
      case 'function_expression':
        this.analyzeFunctionDataFlow(node, content, filePath);
        break;
        
      case 'variable_declaration':
      case 'lexical_declaration':
        this.analyzeVariableDeclaration(node, content, filePath);
        break;
        
      case 'assignment_expression':
        this.analyzeAssignment(node, content, filePath);
        break;
        
      case 'call_expression':
        this.analyzeCallDataFlow(node, content, filePath);
        break;
        
      case 'return_statement':
        this.analyzeReturnStatement(node, content, filePath);
        break;
        
      case 'member_expression':
        this.analyzePropertyAccess(node, content, filePath);
        break;
        
      case 'subscript_expression':
        this.analyzeArrayAccess(node, content, filePath);
        break;
        
      case 'object_pattern':
      case 'array_pattern':
        this.analyzeDestructuring(node, content, filePath);
        break;
        
      case 'if_statement':
      case 'conditional_expression':
        this.analyzeConditionalFlow(node, content, filePath);
        break;
    }

    // Recursively analyze children
    for (let i = 0; i < node.childCount; i++) {
      this.traverseForDataFlow(node.child(i)!, content, filePath);
    }
  }

  private analyzeFunctionDataFlow(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const nameNode = this.findChildByType(node, 'identifier');
    const functionName = nameNode ? this.getNodeText(nameNode, content) : 'anonymous';
    const line = node.startPosition.row + 1;

    // Push function scope
    this.currentFunction = `${filePath}:${functionName}:${line}`;
    this.scopeStack.push(this.currentFunction);

    // Analyze parameters
    const params = this.findChildByType(node, 'formal_parameters');
    if (params) {
      this.analyzeParameters(params, content, filePath, functionName);
    }

    // Analyze function body
    const body = this.findChildByType(node, 'statement_block');
    if (body) {
      for (let i = 0; i < body.childCount; i++) {
        this.traverseForDataFlow(body.child(i)!, content, filePath);
      }
    }

    // Pop function scope
    this.scopeStack.pop();
    if (this.scopeStack.length > 0) {
      this.currentFunction = this.scopeStack[this.scopeStack.length - 1];
    } else {
      this.currentFunction = undefined;
    }
  }

  private analyzeParameters(
    paramsNode: Parser.SyntaxNode,
    content: string,
    filePath: string,
    functionName: string
  ): void {
    for (let i = 0; i < paramsNode.childCount; i++) {
      const param = paramsNode.child(i)!;
      
      if (param.type === 'identifier') {
        const paramName = this.getNodeText(param, content);
        const line = param.startPosition.row + 1;
        
        const paramNode: DataFlowNode = {
          id: `${filePath}:${paramName}:${line}:param`,
          type: 'parameter',
          name: paramName,
          filePath,
          line,
          column: param.startPosition.column,
          scope: this.currentFunction || 'global',
          dataType: this.inferParameterType(param, content)
        };
        
        this.dataFlowNodes.set(paramNode.id, paramNode);
        
        // Track parameter state
        this.trackVariableState(paramName, paramNode, true);
      }
    }
  }

  private analyzeVariableDeclaration(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const declarators = this.findChildrenByType(node, 'variable_declarator');
    
    declarators.forEach(declarator => {
      const nameNode = this.findChildByType(declarator, 'identifier');
      const initNode = declarator.child(2); // Assignment value
      
      if (nameNode) {
        const varName = this.getNodeText(nameNode, content);
        const line = nameNode.startPosition.row + 1;
        
        const varNode: DataFlowNode = {
          id: `${filePath}:${varName}:${line}:var`,
          type: 'variable',
          name: varName,
          filePath,
          line,
          column: nameNode.startPosition.column,
          scope: this.currentFunction || 'global',
          dataType: this.inferVariableType(declarator, content),
          value: this.extractLiteralValue(initNode, content)
        };
        
        this.dataFlowNodes.set(varNode.id, varNode);
        this.trackVariableState(varName, varNode, false);
        
        // If there's an initializer, create data flow edge
        if (initNode) {
          this.analyzeDataFlowFromExpression(initNode, varNode.id, content, filePath);
        }
      }
    });
  }

  private analyzeAssignment(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const leftNode = node.child(0);  // Assignment target
    const rightNode = node.child(2); // Assignment value
    
    if (!leftNode || !rightNode) return;
    
    const targetVar = this.extractVariableFromNode(leftNode, content);
    if (targetVar) {
      const line = node.startPosition.row + 1;
      
      const assignNode: DataFlowNode = {
        id: `${filePath}:${targetVar}:${line}:assign`,
        type: 'variable',
        name: targetVar,
        filePath,
        line,
        column: node.startPosition.column,
        scope: this.currentFunction || 'global'
      };
      
      this.dataFlowNodes.set(assignNode.id, assignNode);
      
      // Track assignment in variable state
      const varState = this.variableStates.get(targetVar);
      if (varState) {
        varState.assignments.push(line);
      }
      
      // Analyze data flow from right side
      this.analyzeDataFlowFromExpression(rightNode, assignNode.id, content, filePath);
    }
  }

  private analyzeCallDataFlow(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const calleeNode = node.child(0);
    const argsNode = this.findChildByType(node, 'arguments');
    
    if (!calleeNode || !argsNode) return;
    
    const functionName = this.extractFunctionName(calleeNode, content);
    const line = node.startPosition.row + 1;
    
    // Create node for function call
    const callNode: DataFlowNode = {
      id: `${filePath}:${functionName}:${line}:call`,
      type: 'function_call',
      name: functionName,
      filePath,
      line,
      column: node.startPosition.column,
      scope: this.currentFunction || 'global'
    };
    
    this.dataFlowNodes.set(callNode.id, callNode);
    
    // Analyze arguments - data flows from variables to function parameters
    this.analyzeCallArguments(argsNode, callNode.id, content, filePath);
    
    // If this is an assignment to a variable, create flow from call result
    const parent = node.parent;
    if (parent && parent.type === 'variable_declarator') {
      const varName = this.extractVariableFromNode(parent.child(0)!, content);
      if (varName) {
        this.createDataFlowEdge(
          callNode.id,
          `${filePath}:${varName}:${line}:var`,
          'return_value',
          filePath
        );
      }
    }
  }

  private analyzeReturnStatement(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const returnValue = node.child(1); // Skip 'return' keyword
    if (!returnValue || !this.currentFunction) return;
    
    const line = node.startPosition.row + 1;
    
    const returnNode: DataFlowNode = {
      id: `${this.currentFunction}:return:${line}`,
      type: 'return',
      name: 'return',
      filePath,
      line,
      column: node.startPosition.column,
      scope: this.currentFunction
    };
    
    this.dataFlowNodes.set(returnNode.id, returnNode);
    
    // Analyze data flow from return expression
    this.analyzeDataFlowFromExpression(returnValue, returnNode.id, content, filePath);
  }

  private analyzePropertyAccess(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const objectNode = node.child(0);
    const propertyNode = node.child(2);
    
    if (!objectNode || !propertyNode) return;
    
    const objectVar = this.extractVariableFromNode(objectNode, content);
    const propertyName = this.getNodeText(propertyNode, content);
    
    if (objectVar) {
      const line = node.startPosition.row + 1;
      
      const propNode: DataFlowNode = {
        id: `${filePath}:${objectVar}.${propertyName}:${line}:prop`,
        type: 'property',
        name: `${objectVar}.${propertyName}`,
        filePath,
        line,
        column: node.startPosition.column,
        scope: this.currentFunction || 'global'
      };
      
      this.dataFlowNodes.set(propNode.id, propNode);
      
      // Create data flow edge from object to property
      const objectState = this.variableStates.get(objectVar);
      if (objectState) {
        this.createDataFlowEdge(
          `${filePath}:${objectVar}:${objectState.assignments[0] || 1}:var`,
          propNode.id,
          'property_access',
          filePath
        );
      }
    }
  }

  private analyzeArrayAccess(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    const arrayNode = node.child(0);
    const indexNode = node.child(2);
    
    if (!arrayNode) return;
    
    const arrayVar = this.extractVariableFromNode(arrayNode, content);
    const indexValue = indexNode ? this.getNodeText(indexNode, content) : '?';
    
    if (arrayVar) {
      const line = node.startPosition.row + 1;
      
      const elemNode: DataFlowNode = {
        id: `${filePath}:${arrayVar}[${indexValue}]:${line}:elem`,
        type: 'property',
        name: `${arrayVar}[${indexValue}]`,
        filePath,
        line,
        column: node.startPosition.column,
        scope: this.currentFunction || 'global'
      };
      
      this.dataFlowNodes.set(elemNode.id, elemNode);
      
      // Create data flow edge from array to element
      const arrayState = this.variableStates.get(arrayVar);
      if (arrayState) {
        this.createDataFlowEdge(
          `${filePath}:${arrayVar}:${arrayState.assignments[0] || 1}:var`,
          elemNode.id,
          'array_access',
          filePath
        );
      }
    }
  }

  private analyzeDestructuring(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    // Analyze destructuring assignments like {a, b} = obj or [x, y] = arr
    const parent = node.parent;
    if (!parent || parent.type !== 'variable_declarator') return;
    
    const sourceNode = parent.child(2); // The value being destructured
    if (!sourceNode) return;
    
    const sourceVar = this.extractVariableFromNode(sourceNode, content);
    if (!sourceVar) return;
    
    // Extract destructured variables
    const destructuredVars = this.extractDestructuredVariables(node, content);
    
    destructuredVars.forEach(varName => {
      const line = node.startPosition.row + 1;
      
      const destNode: DataFlowNode = {
        id: `${filePath}:${varName}:${line}:dest`,
        type: 'variable',
        name: varName,
        filePath,
        line,
        column: node.startPosition.column,
        scope: this.currentFunction || 'global'
      };
      
      this.dataFlowNodes.set(destNode.id, destNode);
      
      // Create data flow edge from source to destructured variable
      const sourceState = this.variableStates.get(sourceVar);
      if (sourceState) {
        this.createDataFlowEdge(
          `${filePath}:${sourceVar}:${sourceState.assignments[0] || 1}:var`,
          destNode.id,
          'destructuring',
          filePath
        );
      }
    });
  }

  private analyzeConditionalFlow(
    node: Parser.SyntaxNode,
    content: string,
    filePath: string
  ): void {
    // Track data flow through conditional statements
    const conditionNode = node.child(2); // Skip 'if' and '('
    if (!conditionNode) return;
    
    // Analyze variables used in condition
    const conditionVars = this.extractVariablesFromExpression(conditionNode, content);
    
    // Analyze then/else branches for data flow
    const thenNode = this.findChildByType(node, 'statement_block');
    const elseNode = node.children.find(child => child.type === 'else_clause');
    
    // Track conditional data flows
    conditionVars.forEach(varName => {
      const varState = this.variableStates.get(varName);
      if (varState) {
        const line = conditionNode.startPosition.row + 1;
        varState.accesses.push(line);
        
        // Mark variables assigned in branches as conditionally flowing
        if (thenNode) {
          this.markConditionalAssignments(thenNode, varName, filePath);
        }
        if (elseNode) {
          this.markConditionalAssignments(elseNode, varName, filePath);
        }
      }
    });
  }

  // Helper methods

  private analyzeDataFlowFromExpression(
    expr: Parser.SyntaxNode,
    targetNodeId: string,
    content: string,
    filePath: string
  ): void {
    const sourceVars = this.extractVariablesFromExpression(expr, content);
    
    sourceVars.forEach(varName => {
      const varState = this.variableStates.get(varName);
      if (varState) {
        const sourceNodeId = `${filePath}:${varName}:${varState.assignments[varState.assignments.length - 1] || 1}:var`;
        this.createDataFlowEdge(sourceNodeId, targetNodeId, 'assignment', filePath);
      }
    });
  }

  private analyzeCallArguments(
    argsNode: Parser.SyntaxNode,
    callNodeId: string,
    content: string,
    filePath: string
  ): void {
    for (let i = 0; i < argsNode.childCount; i++) {
      const arg = argsNode.child(i)!;
      if (arg.type !== ',' && arg.type !== '(' && arg.type !== ')') {
        const argVars = this.extractVariablesFromExpression(arg, content);
        
        argVars.forEach(varName => {
          const varState = this.variableStates.get(varName);
          if (varState) {
            const sourceNodeId = `${filePath}:${varName}:${varState.assignments[varState.assignments.length - 1] || 1}:var`;
            this.createDataFlowEdge(sourceNodeId, callNodeId, 'parameter_passing', filePath);
          }
        });
      }
    }
  }

  private createDataFlowEdge(
    fromNodeId: string,
    toNodeId: string,
    flowType: DataFlowType,
    filePath: string,
    transformation?: string
  ): void {
    const edgeId = `${fromNodeId}:${flowType}:${toNodeId}`;
    
    const edge: DataFlowEdge = {
      id: edgeId,
      from: fromNodeId,
      to: toNodeId,
      type: flowType,
      transformation,
      strength: this.calculateFlowStrength(flowType),
      conditions: []
    };
    
    this.dataFlowEdges.set(edgeId, edge);
  }

  private trackVariableState(varName: string, node: DataFlowNode, isParameter: boolean): void {
    if (!this.variableStates.has(varName)) {
      this.variableStates.set(varName, {
        name: varName,
        scope: node.scope,
        assignments: [],
        accesses: [],
        flowsTo: new Set(),
        flowsFrom: new Set(),
        isParameter,
        isReturnValue: false,
        isCapturedInClosure: false
      });
    }
    
    const state = this.variableStates.get(varName)!;
    if (!isParameter) {
      state.assignments.push(node.line);
    }
  }

  private generateDataFlowRelationships(): CodeRelationship[] {
    const relationships: CodeRelationship[] = [];
    
    for (const edge of this.dataFlowEdges.values()) {
      const fromNode = this.dataFlowNodes.get(edge.from);
      const toNode = this.dataFlowNodes.get(edge.to);
      
      if (fromNode && toNode) {
        const relationship: CodeRelationship = {
          id: `data_flow:${edge.id}`,
          fromSymbol: edge.from,
          toSymbol: edge.to,
          type: 'data_flow',
          strength: edge.strength,
          confidence: 0.8,
          metadata: {
            flowType: edge.type,
            transformations: edge.transformation ? [edge.transformation] : undefined,
            conditions: edge.conditions,
            sourceLocation: {
              line: fromNode.line,
              column: fromNode.column
            }
          }
        };
        
        relationships.push(relationship);
      }
    }
    
    return relationships;
  }

  // Utility methods

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

  private extractVariableFromNode(node: Parser.SyntaxNode, content: string): string | null {
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

  private extractVariablesFromExpression(node: Parser.SyntaxNode, content: string): string[] {
    const variables: string[] = [];
    
    const traverse = (n: Parser.SyntaxNode): void => {
      if (n.type === 'identifier') {
        const name = this.getNodeText(n, content);
        if (!['true', 'false', 'null', 'undefined'].includes(name)) {
          variables.push(name);
        }
      }
      
      for (let i = 0; i < n.childCount; i++) {
        traverse(n.child(i)!);
      }
    };
    
    traverse(node);
    return [...new Set(variables)]; // Remove duplicates
  }

  private extractFunctionName(node: Parser.SyntaxNode, content: string): string {
    if (node.type === 'identifier') {
      return this.getNodeText(node, content);
    }
    
    if (node.type === 'member_expression') {
      const property = node.child(2);
      return property ? this.getNodeText(property, content) : 'unknown';
    }
    
    return 'anonymous';
  }

  private extractDestructuredVariables(node: Parser.SyntaxNode, content: string): string[] {
    const variables: string[] = [];
    
    if (node.type === 'object_pattern') {
      const properties = this.findChildrenByType(node, 'shorthand_property_identifier');
      properties.forEach(prop => {
        variables.push(this.getNodeText(prop, content));
      });
    } else if (node.type === 'array_pattern') {
      for (let i = 0; i < node.childCount; i++) {
        const child = node.child(i)!;
        if (child.type === 'identifier') {
          variables.push(this.getNodeText(child, content));
        }
      }
    }
    
    return variables;
  }

  private markConditionalAssignments(
    branchNode: Parser.SyntaxNode,
    conditionVar: string,
    filePath: string
  ): void {
    // Implementation to mark variables assigned in conditional branches
    // This would traverse the branch and mark assignments as conditional flows
  }

  private inferParameterType(node: Parser.SyntaxNode, content: string): string | undefined {
    // Look for type annotations
    const typeAnnotation = this.findChildByType(node.parent!, 'type_annotation');
    return typeAnnotation ? this.getNodeText(typeAnnotation, content) : undefined;
  }

  private inferVariableType(node: Parser.SyntaxNode, content: string): string | undefined {
    // Infer type from value or type annotation
    const typeAnnotation = this.findChildByType(node, 'type_annotation');
    if (typeAnnotation) {
      return this.getNodeText(typeAnnotation, content);
    }
    
    // Simple type inference from literal values
    const initValue = node.child(2);
    if (initValue) {
      const text = this.getNodeText(initValue, content);
      if (text.startsWith('"') || text.startsWith("'")) return 'string';
      if (text.match(/^\d+$/)) return 'number';
      if (text === 'true' || text === 'false') return 'boolean';
      if (text.startsWith('[')) return 'array';
      if (text.startsWith('{')) return 'object';
    }
    
    return undefined;
  }

  private extractLiteralValue(node: Parser.SyntaxNode | null, content: string): string | undefined {
    if (!node) return undefined;
    
    if (['string', 'number', 'true', 'false', 'null'].includes(node.type)) {
      return this.getNodeText(node, content);
    }
    
    return undefined;
  }

  private calculateFlowStrength(flowType: DataFlowType): number {
    const strengthMap: Record<DataFlowType, number> = {
      assignment: 0.9,
      parameter_passing: 0.8,
      return_value: 0.8,
      property_access: 0.7,
      array_access: 0.7,
      destructuring: 0.8,
      spread: 0.6,
      closure_capture: 0.7,
      callback_parameter: 0.6,
      promise_resolution: 0.5,
      conditional_flow: 0.4
    };
    
    return strengthMap[flowType] || 0.5;
  }

  // Public methods for analysis results

  getDataFlowPaths(startVariable: string, endVariable: string): DataFlowPath[] {
    // Implementation to find all data flow paths between two variables
    return [];
  }

  getVariableUsages(variableName: string): VariableState | undefined {
    return this.variableStates.get(variableName);
  }

  getDataFlowStatistics() {
    return {
      totalNodes: this.dataFlowNodes.size,
      totalEdges: this.dataFlowEdges.size,
      totalVariables: this.variableStates.size,
      flowTypeDistribution: this.getFlowTypeDistribution()
    };
  }

  private getFlowTypeDistribution(): Map<DataFlowType, number> {
    const distribution = new Map<DataFlowType, number>();
    
    for (const edge of this.dataFlowEdges.values()) {
      const count = distribution.get(edge.type) || 0;
      distribution.set(edge.type, count + 1);
    }
    
    return distribution;
  }
}