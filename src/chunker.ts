import { CodeChunk, ChunkType } from './types';
import { FileChange } from './git-scanner';

export class SmartChunker {
  async chunkFile(
    filePath: string, 
    content: string, 
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): Promise<CodeChunk[]> {
    const language = this.detectLanguage(filePath);
    
    if (this.isCodeFile(filePath)) {
      return await this.chunkCodeFile(filePath, content, language, fileChange, coChangeFiles);
    } else {
      return await this.chunkDocumentFile(filePath, content, fileChange, coChangeFiles);
    }
  }

  private async chunkCodeFile(
    filePath: string,
    content: string,
    language: string,
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): Promise<CodeChunk[]> {
    const chunks: CodeChunk[] = [];
    const lines = content.split('\n');

    if (language === 'javascript' || language === 'typescript') {
      return this.chunkJavaScriptTypeScript(filePath, content, lines, fileChange, coChangeFiles);
    } else if (language === 'python') {
      return this.chunkPython(filePath, content, lines, fileChange, coChangeFiles);
    } else {
      // Generic chunking for other languages
      return this.chunkGeneric(filePath, content, lines, fileChange, coChangeFiles);
    }
  }

  private chunkJavaScriptTypeScript(
    filePath: string,
    content: string,
    lines: string[],
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): CodeChunk[] {
    const chunks: CodeChunk[] = [];
    let currentChunk = '';
    let chunkStartLine = 1;
    let chunkType: ChunkType = 'function';
    let symbolName = '';
    let braceCount = 0;
    let inFunction = false;
    let inClass = false;

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const trimmedLine = line.trim();

      // Detect function declarations
      if (this.isFunctionDeclaration(trimmedLine)) {
        // Save previous chunk if exists
        if (currentChunk && inFunction) {
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i, 
            chunkType, symbolName, fileChange, coChangeFiles
          ));
        }
        
        // Start new function chunk
        inFunction = true;
        chunkType = 'function';
        symbolName = this.extractFunctionName(trimmedLine);
        chunkStartLine = i + 1;
        currentChunk = line + '\n';
        braceCount = (line.match(/{/g) || []).length - (line.match(/}/g) || []).length;
      }
      // Detect class declarations
      else if (this.isClassDeclaration(trimmedLine)) {
        // Save previous chunk if exists
        if (currentChunk && (inFunction || inClass)) {
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i, 
            chunkType, symbolName, fileChange, coChangeFiles
          ));
        }
        
        // Start new class chunk
        inClass = true;
        chunkType = 'class';
        symbolName = this.extractClassName(trimmedLine);
        chunkStartLine = i + 1;
        currentChunk = line + '\n';
        braceCount = (line.match(/{/g) || []).length - (line.match(/}/g) || []).length;
      }
      else if (inFunction || inClass) {
        currentChunk += line + '\n';
        braceCount += (line.match(/{/g) || []).length - (line.match(/}/g) || []).length;
        
        // Check if function/class is complete
        if (braceCount <= 0 && currentChunk.trim()) {
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i + 1, 
            chunkType, symbolName, fileChange, coChangeFiles
          ));
          
          currentChunk = '';
          inFunction = false;
          inClass = false;
          braceCount = 0;
        }
      }
      // Handle top-level code (imports, exports, constants)
      else if (this.isTopLevelStatement(trimmedLine)) {
        chunks.push(this.createChunk(
          filePath, line, i + 1, i + 1, 
          'config', this.extractImportExport(trimmedLine), fileChange, coChangeFiles
        ));
      }
    }

    // Handle any remaining chunk
    if (currentChunk && (inFunction || inClass)) {
      chunks.push(this.createChunk(
        filePath, currentChunk, chunkStartLine, lines.length, 
        chunkType, symbolName, fileChange, coChangeFiles
      ));
    }

    return chunks.length > 0 ? chunks : [this.createChunk(
      filePath, content, 1, lines.length, 'function', '', fileChange, coChangeFiles
    )];
  }

  private chunkPython(
    filePath: string,
    content: string,
    lines: string[],
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): CodeChunk[] {
    const chunks: CodeChunk[] = [];
    let currentChunk = '';
    let chunkStartLine = 1;
    let chunkType: ChunkType = 'function';
    let symbolName = '';
    let indentLevel = 0;
    let inFunction = false;
    let inClass = false;

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const trimmedLine = line.trim();
      const currentIndent = line.length - line.trimStart().length;

      // Detect function definitions
      if (trimmedLine.startsWith('def ')) {
        // Save previous chunk if exists
        if (currentChunk && (inFunction || inClass)) {
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i, 
            chunkType, symbolName, fileChange, coChangeFiles
          ));
        }
        
        inFunction = true;
        chunkType = 'function';
        symbolName = this.extractPythonFunctionName(trimmedLine);
        chunkStartLine = i + 1;
        currentChunk = line + '\n';
        indentLevel = currentIndent;
      }
      // Detect class definitions
      else if (trimmedLine.startsWith('class ')) {
        // Save previous chunk if exists
        if (currentChunk && (inFunction || inClass)) {
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i, 
            chunkType, symbolName, fileChange, coChangeFiles
          ));
        }
        
        inClass = true;
        chunkType = 'class';
        symbolName = this.extractPythonClassName(trimmedLine);
        chunkStartLine = i + 1;
        currentChunk = line + '\n';
        indentLevel = currentIndent;
      }
      else if (inFunction || inClass) {
        // Check if we're still in the same function/class based on indentation
        if (trimmedLine && currentIndent <= indentLevel && !trimmedLine.startsWith('#')) {
          // End of current function/class
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i, 
            chunkType, symbolName, fileChange, coChangeFiles
          ));
          
          currentChunk = '';
          inFunction = false;
          inClass = false;
          
          // Check if this line starts a new function/class
          if (trimmedLine.startsWith('def ') || trimmedLine.startsWith('class ')) {
            i--; // Reprocess this line
            continue;
          }
        } else {
          currentChunk += line + '\n';
        }
      }
      // Handle top-level imports and assignments
      else if (this.isPythonTopLevel(trimmedLine)) {
        chunks.push(this.createChunk(
          filePath, line, i + 1, i + 1, 
          'config', trimmedLine.split(' ')[0], fileChange, coChangeFiles
        ));
      }
    }

    // Handle any remaining chunk
    if (currentChunk && (inFunction || inClass)) {
      chunks.push(this.createChunk(
        filePath, currentChunk, chunkStartLine, lines.length, 
        chunkType, symbolName, fileChange, coChangeFiles
      ));
    }

    return chunks.length > 0 ? chunks : [this.createChunk(
      filePath, content, 1, lines.length, 'function', '', fileChange, coChangeFiles
    )];
  }

  private chunkGeneric(
    filePath: string,
    content: string,
    lines: string[],
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): CodeChunk[] {
    // Simple line-based chunking for unknown languages
    const maxLinesPerChunk = 50;
    const chunks: CodeChunk[] = [];
    
    for (let i = 0; i < lines.length; i += maxLinesPerChunk) {
      const chunkLines = lines.slice(i, i + maxLinesPerChunk);
      const chunkContent = chunkLines.join('\n');
      
      chunks.push(this.createChunk(
        filePath, chunkContent, i + 1, Math.min(i + maxLinesPerChunk, lines.length),
        'function', `chunk_${Math.floor(i / maxLinesPerChunk)}`, fileChange, coChangeFiles
      ));
    }
    
    return chunks;
  }

  private async chunkDocumentFile(
    filePath: string,
    content: string,
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): Promise<CodeChunk[]> {
    const chunks: CodeChunk[] = [];
    
    if (filePath.endsWith('.md')) {
      return this.chunkMarkdown(filePath, content, fileChange, coChangeFiles);
    } else {
      // Simple paragraph-based chunking for other docs
      const paragraphs = content.split('\n\n').filter(p => p.trim());
      let lineNumber = 1;
      
      for (let i = 0; i < paragraphs.length; i++) {
        const paragraph = paragraphs[i];
        const endLine = lineNumber + paragraph.split('\n').length - 1;
        
        chunks.push(this.createChunk(
          filePath, paragraph, lineNumber, endLine,
          'documentation', `section_${i}`, fileChange, coChangeFiles
        ));
        
        lineNumber = endLine + 2; // Account for empty line
      }
    }
    
    return chunks;
  }

  private chunkMarkdown(
    filePath: string,
    content: string,
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): CodeChunk[] {
    const chunks: CodeChunk[] = [];
    const lines = content.split('\n');
    let currentChunk = '';
    let chunkStartLine = 1;
    let symbolName = '';

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      
      // Detect headers
      if (line.startsWith('#')) {
        // Save previous chunk if exists
        if (currentChunk.trim()) {
          chunks.push(this.createChunk(
            filePath, currentChunk, chunkStartLine, i, 
            'documentation', symbolName, fileChange, coChangeFiles
          ));
        }
        
        // Start new section
        symbolName = line.replace(/^#+\s*/, '');
        chunkStartLine = i + 1;
        currentChunk = line + '\n';
      } else {
        currentChunk += line + '\n';
      }
    }

    // Handle final chunk
    if (currentChunk.trim()) {
      chunks.push(this.createChunk(
        filePath, currentChunk, chunkStartLine, lines.length, 
        'documentation', symbolName, fileChange, coChangeFiles
      ));
    }

    return chunks.length > 0 ? chunks : [this.createChunk(
      filePath, content, 1, lines.length, 'documentation', '', fileChange, coChangeFiles
    )];
  }

  private createChunk(
    filePath: string,
    content: string,
    startLine: number,
    endLine: number,
    chunkType: ChunkType,
    symbolName: string,
    fileChange?: FileChange,
    coChangeFiles: string[] = []
  ): CodeChunk {
    return {
      chunk_id: this.generateChunkId(filePath, startLine),
      file_path: filePath,
      symbol_name: symbolName || undefined,
      chunk_type: chunkType,
      start_line: startLine,
      end_line: endLine,
      content: content.trim(),
      content_hash: this.hashContent(content),
      embedding: [], // Will be filled by embedder
      relationships: {
        calls: this.extractCalls(content),
        called_by: [],
        imports: this.extractImports(content),
        exports: this.extractExports(content),
        data_flow: []
      },
      git_metadata: {
        last_modified_commit: fileChange?.lastCommit || '',
        commit_author: fileChange?.commitAuthor || '',
        commit_message: fileChange?.commitMessage || '',
        commit_date: fileChange?.commitDate || new Date().toISOString(),
        file_history_length: 1,
        co_change_files: coChangeFiles
      },
      language_metadata: {
        language: this.detectLanguage(filePath),
        complexity_score: this.calculateComplexity(content),
        dependencies: this.extractImports(content),
        exports: this.extractExports(content)
      },
      usage_patterns: {
        access_frequency: 0,
        task_contexts: []
      },
      last_modified: fileChange?.commitDate || new Date().toISOString()
    };
  }

  private detectChunkType(filePath: string): ChunkType {
    if (filePath.endsWith('.md') || filePath.endsWith('.txt')) {
      return 'documentation';
    }
    if (filePath.endsWith('.json') || filePath.endsWith('.yaml')) {
      return 'config';
    }
    return 'function'; // Default for code files
  }

  // Helper methods for language detection and parsing
  private isCodeFile(filePath: string): boolean {
    const ext = this.getFileExtension(filePath);
    const codeExtensions = ['.js', '.jsx', '.ts', '.tsx', '.py', '.go', '.rs', '.java', '.cpp', '.c', '.h'];
    return codeExtensions.includes(ext);
  }

  private getFileExtension(filePath: string): string {
    return filePath.substring(filePath.lastIndexOf('.')).toLowerCase();
  }

  private isFunctionDeclaration(line: string): boolean {
    return /^(function\s+\w+|const\s+\w+\s*=\s*(?:\([^)]*\)\s*)?=>|export\s+(?:function|const)\s+\w+|async\s+function\s+\w+)/.test(line) ||
           /^\w+\s*\([^)]*\)\s*\{/.test(line) || // Method in class
           line.includes(') {') && (line.includes('function') || line.includes('=>'));
  }

  private isClassDeclaration(line: string): boolean {
    return /^(class\s+\w+|export\s+class\s+\w+)/.test(line);
  }

  private isTopLevelStatement(line: string): boolean {
    return /^(import\s+|export\s+|const\s+\w+\s*=|let\s+\w+\s*=|var\s+\w+\s*=)/.test(line);
  }

  private extractFunctionName(line: string): string {
    const matches = line.match(/(?:function\s+(\w+)|const\s+(\w+)|export\s+(?:function\s+)?(\w+)|(\w+)\s*\()/);
    return matches ? (matches[1] || matches[2] || matches[3] || matches[4]) : '';
  }

  private extractClassName(line: string): string {
    const matches = line.match(/class\s+(\w+)/);
    return matches ? matches[1] : '';
  }

  private extractImportExport(line: string): string {
    const matches = line.match(/(?:import|export).*?(\w+)/);
    return matches ? matches[1] : '';
  }

  private extractPythonFunctionName(line: string): string {
    const matches = line.match(/def\s+(\w+)/);
    return matches ? matches[1] : '';
  }

  private extractPythonClassName(line: string): string {
    const matches = line.match(/class\s+(\w+)/);
    return matches ? matches[1] : '';
  }

  private isPythonTopLevel(line: string): boolean {
    return /^(import\s+|from\s+\S+\s+import|^\w+\s*=)/.test(line);
  }

  private extractCalls(content: string): string[] {
    // Simple function call extraction
    const callMatches = content.match(/(\w+)\s*\(/g);
    return callMatches ? callMatches.map(match => match.replace(/\s*\(/, '')) : [];
  }

  private extractImports(content: string): string[] {
    const imports: string[] = [];
    
    // JavaScript/TypeScript imports
    const jsImports = content.match(/import.*?from\s+['"]([^'"]+)['"]/g);
    if (jsImports) {
      imports.push(...jsImports.map(imp => imp.match(/from\s+['"]([^'"]+)['"]/)![1]));
    }

    // Python imports
    const pyImports = content.match(/(?:import\s+(\w+)|from\s+(\w+)\s+import)/g);
    if (pyImports) {
      imports.push(...pyImports.map(imp => {
        const match = imp.match(/(?:import\s+(\w+)|from\s+(\w+))/);
        return match ? (match[1] || match[2]) : '';
      }).filter(Boolean));
    }

    return imports;
  }

  private extractExports(content: string): string[] {
    const exports: string[] = [];
    
    // JavaScript/TypeScript exports
    const jsExports = content.match(/export\s+(?:function\s+(\w+)|const\s+(\w+)|class\s+(\w+))/g);
    if (jsExports) {
      exports.push(...jsExports.map(exp => {
        const match = exp.match(/export\s+(?:function\s+(\w+)|const\s+(\w+)|class\s+(\w+))/);
        return match ? (match[1] || match[2] || match[3]) : '';
      }).filter(Boolean));
    }

    return exports;
  }

  private calculateComplexity(content: string): number {
    // Simple cyclomatic complexity calculation
    const wordKeywords = [
      'if', 'else', 'elif', 'while', 'for', 'switch', 'case', 
      'try', 'catch', 'except'
    ];
    
    const operatorKeywords = [
      '&&', '\\|\\|', '\\?'  // Escape special regex characters
    ];
    
    let complexity = 1; // Base complexity
    
    // Handle word-based keywords with word boundaries
    for (const keyword of wordKeywords) {
      const regex = new RegExp(`\\b${keyword}\\b`, 'g');
      const matches = content.match(regex);
      if (matches) {
        complexity += matches.length;
      }
    }
    
    // Handle operator keywords without word boundaries
    for (const operator of operatorKeywords) {
      const regex = new RegExp(operator, 'g');
      const matches = content.match(regex);
      if (matches) {
        complexity += matches.length;
      }
    }
    
    return complexity;
  }

  private detectLanguage(filePath: string): string {
    const ext = this.getFileExtension(filePath);
    const langMap: Record<string, string> = {
      '.ts': 'typescript',
      '.tsx': 'typescript',
      '.js': 'javascript',
      '.jsx': 'javascript',
      '.py': 'python',
      '.go': 'go',
      '.rs': 'rust',
      '.java': 'java',
      '.cpp': 'cpp',
      '.c': 'c',
      '.h': 'c',
      '.md': 'markdown',
      '.json': 'json',
      '.yaml': 'yaml',
      '.yml': 'yaml'
    };
    return langMap[ext] || 'unknown';
  }

  private generateChunkId(filePath: string, startLine: number): string {
    return `${filePath}:${startLine}`;
  }

  private hashContent(content: string): string {
    // Simple hash function - in production use crypto.createHash
    let hash = 0;
    for (let i = 0; i < content.length; i++) {
      const char = content.charCodeAt(i);
      hash = ((hash << 5) - hash) + char;
      hash = hash & hash; // Convert to 32-bit integer
    }
    return hash.toString(16);
  }
}