import { readFileSync, writeFileSync, existsSync, renameSync } from 'fs';
import { join } from 'path';
import type { ProjectContextInfo, ImplementationPatterns } from '../types/patterns.js';

export class CLAUDEMdMaintainer {
  private projectRoot: string;
  private claudeFilePath: string;

  constructor(projectRoot: string = process.cwd()) {
    this.projectRoot = projectRoot;
    this.claudeFilePath = join(projectRoot, 'CLAUDE.md');
  }

  /**
   * Update CLAUDE.md with new project context if there are meaningful changes
   */
  async updateProjectContext(newContext: ProjectContextInfo): Promise<void> {
    try {
      const claudeContent = await this.readCLAUDEMd();
      const currentSection = this.extractProjectContextSection(claudeContent);
      const newSection = this.generateProjectContextSection(newContext);
      
      // Only update if there are meaningful changes
      if (this.hasSignificantChanges(currentSection, newSection)) {
        const updatedContent = this.insertOrUpdateSection(claudeContent, newSection);
        await this.atomicWrite(this.claudeFilePath, updatedContent);
        console.log('✅ CLAUDE.md updated with current project context and implementation patterns');
        return;
      }
      
      console.log('📋 CLAUDE.md is up to date - no significant changes detected');
    } catch (error) {
      console.error('❌ Failed to update CLAUDE.md:', error);
      throw error;
    }
  }

  /**
   * Generate the auto-maintained project context section
   */
  private generateProjectContextSection(context: ProjectContextInfo): string {
    // Safe array handling
    const directories = Array.isArray(context.directories) ? context.directories : [];
    const dependencies = Array.isArray(context.dependencies) ? context.dependencies : [];
    const scripts = context.scripts || {};
    const impl = context.implementationPatterns;
    
    const directoriesText = directories
      .map(d => `  - ${d.path}/ (${d.purpose})`)
      .join('\n');
      
    const authSection = this.generateAuthSection(impl.authentication);
    const apiSection = this.generateApiSection(impl.apiResponses);
    const errorSection = this.generateErrorSection(impl.errorHandling);
    const guardrailsSection = this.generateGuardrailsSection(impl);
      
    return `<!-- claudecat:auto:begin:project-context -->
## Project Context (Auto-Maintained by ClaudeCat)

**Project Type**: ${context.projectType}  
**Language**: ${context.language}  
**Framework**: ${context.framework}  
**Package Manager**: ${context.packageManager}

### Implementation Patterns

${authSection}

${apiSection}

${errorSection}

### Development Information

**Scripts**:
- dev: \`${scripts.dev || 'Not detected'}\`
- build: \`${scripts.build || 'Not detected'}\`
- test: \`${scripts.test || 'Not detected'}\`

**Key Directories**:
${directoriesText || '  - No standard directories detected'}

**Core Dependencies**: ${dependencies.slice(0, 10).join(', ')}${dependencies.length > 10 ? '...' : ''}

${guardrailsSection}

**Last Updated**: ${context.lastUpdated}  
**Detection Quality**: Implementation patterns auto-detected with confidence scoring

*This section is automatically maintained by ClaudeCat. All patterns include confidence scores and evidence citations.*
<!-- claudecat:auto:end:project-context -->

`;
  }

  /**
   * Generate authentication implementation section
   */
  private generateAuthSection(auth: ImplementationPatterns['authentication']): string {
    const confidenceText = this.getConfidenceText(auth.confidence);
    const evidenceText = auth.evidence.length > 0 ? 
      `\n  Evidence: ${auth.evidence.slice(0, 3).join(', ')}` : '';
    
    return `#### Authentication Implementation (${confidenceText})
- **User Property**: \`${auth.userProperty}\`
- **Token Storage**: ${auth.tokenLocation}
- **Error Response**: ${auth.errorResponse.format}${auth.errorResponse.statusCode ? ` (${auth.errorResponse.statusCode} status)` : ''}
- **Middleware Pattern**: ${auth.middlewarePattern}${evidenceText}${this.getUncertaintyWarning(auth.confidence)}`;
  }

  /**
   * Generate API response implementation section
   */
  private generateApiSection(api: ImplementationPatterns['apiResponses']): string {
    const confidenceText = this.getConfidenceText(api.confidence);
    const evidenceText = api.evidence.length > 0 ? 
      `\n  Evidence: ${api.evidence.slice(0, 3).join(', ')}` : '';
    
    return `#### API Response Implementation (${confidenceText})
- **Success Format**: ${api.successFormat}
- **Error Format**: ${api.errorFormat}
- **Status Codes**: ${api.statusCodeUsage}
- **Wrapper Pattern**: ${api.wrapperPattern}${evidenceText}${this.getUncertaintyWarning(api.confidence)}`;
  }

  /**
   * Generate error handling implementation section
   */
  private generateErrorSection(error: ImplementationPatterns['errorHandling']): string {
    const confidenceText = this.getConfidenceText(error.confidence);
    const evidenceText = error.evidence.length > 0 ? 
      `\n  Evidence: ${error.evidence.slice(0, 3).join(', ')}` : '';
    
    return `#### Error Handling Implementation (${confidenceText})
- **Catch Pattern**: ${error.catchPattern}
- **Error Structure**: ${error.errorStructure}
- **Logging Integration**: ${error.loggingIntegration}
- **Propagation Style**: ${error.propagationStyle}${evidenceText}${this.getUncertaintyWarning(error.confidence)}`;
  }

  /**
   * Generate critical guardrails based on detected patterns
   */
  private generateGuardrailsSection(impl: ImplementationPatterns): string {
    const guardrails: string[] = [];

    // Authentication guardrails
    if (impl.authentication.tokenLocation === 'httpOnly cookie' && impl.authentication.confidence >= 60) {
      guardrails.push('❌ **NEVER use localStorage for tokens** - This project uses httpOnly cookies');
    }
    if (impl.authentication.userProperty !== 'Unknown' && impl.authentication.confidence >= 60) {
      guardrails.push(`✅ **ALWAYS use \`${impl.authentication.userProperty}\`** for authenticated user data`);
    }

    // API response guardrails  
    if (impl.apiResponses.successFormat !== 'Unknown' && impl.apiResponses.confidence >= 60) {
      if (impl.apiResponses.successFormat === 'bare object') {
        guardrails.push('✅ **Use bare object responses** - No wrapper format detected');
      } else {
        guardrails.push(`✅ **ALWAYS wrap API responses** in \`${impl.apiResponses.successFormat}\` format`);
      }
    }
    if (impl.apiResponses.errorFormat !== 'Unknown' && impl.apiResponses.confidence >= 60) {
      guardrails.push(`✅ **Use \`${impl.apiResponses.errorFormat}\`** for error responses`);
    }

    // Error handling guardrails
    if (impl.errorHandling.catchPattern !== 'Unknown' && impl.errorHandling.confidence >= 60) {
      guardrails.push(`✅ **Follow \`${impl.errorHandling.catchPattern}\`** error handling pattern`);
    }

    if (guardrails.length === 0) {
      return '### Critical Guardrails\n\n⚠️ **Implementation patterns unclear** - Verify project conventions before making architectural decisions';
    }

    return `### Critical Guardrails

${guardrails.join('\n')}`;
  }

  /**
   * Get confidence level text description
   */
  private getConfidenceText(confidence: number): string {
    if (confidence >= 80) return `${confidence}% - High Confidence`;
    if (confidence >= 60) return `${confidence}% - Medium Confidence`;
    return `${confidence}% - Low Confidence`;
  }

  /**
   * Generate uncertainty warning for low confidence patterns
   */
  private getUncertaintyWarning(confidence: number): string {
    if (confidence < 60) {
      return '\\n  ⚠️ **Low confidence** - Ask before making assumptions about this pattern';
    } else if (confidence < 80) {
      return '\\n  ⚠️ **Medium confidence** - Verify before critical architectural decisions';
    }
    return '';
  }

  /**
   * Extract the current project context section from CLAUDE.md
   */
  private extractProjectContextSection(content: string): string {
    const markerStart = '<!-- claudecat:auto:begin:project-context -->';
    const markerEnd = '<!-- claudecat:auto:end:project-context -->';
    const markerRegex = new RegExp(`${this.escapeRegex(markerStart)}[\\s\\S]*?${this.escapeRegex(markerEnd)}`, 'i');
    
    const match = content.match(markerRegex);
    return match ? match[0] : '';
  }

  /**
   * Insert or update the project context section in CLAUDE.md
   */
  private insertOrUpdateSection(content: string, newSection: string): string {
    const markerStart = '<!-- claudecat:auto:begin:project-context -->';
    const markerEnd = '<!-- claudecat:auto:end:project-context -->';
    const markerRegex = new RegExp(`${this.escapeRegex(markerStart)}[\\s\\S]*?${this.escapeRegex(markerEnd)}`, 'i');
    
    if (markerRegex.test(content)) {
      // Update existing section between markers
      return content.replace(markerRegex, newSection.trim());
    } else {
      // Insert new section after first header or at the beginning
      const lines = content.split('\\n');
      const headerIndex = lines.findIndex(line => line.startsWith('#'));
      const insertIndex = headerIndex >= 0 ? headerIndex + 1 : 0;
      
      lines.splice(insertIndex, 0, '', newSection.trim(), '');
      return lines.join('\\n');
    }
  }

  /**
   * Check if there are significant changes between current and new context
   */
  private hasSignificantChanges(currentSection: string, newSection: string): boolean {
    // Simple content comparison - normalize whitespace and compare
    const normalize = (text: string) => text.replace(/\\s+/g, ' ').trim();
    
    const currentNormalized = normalize(currentSection);
    const newNormalized = normalize(newSection);
    
    return currentNormalized !== newNormalized;
  }

  /**
   * Read CLAUDE.md file or create basic template if it doesn't exist
   */
  private async readCLAUDEMd(): Promise<string> {
    if (!existsSync(this.claudeFilePath)) {
      const basicTemplate = `# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`;
      return basicTemplate;
    }
    
    try {
      return readFileSync(this.claudeFilePath, 'utf8');
    } catch (error) {
      console.warn('⚠️ Could not read CLAUDE.md, creating new file');
      return `# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`;
    }
  }

  /**
   * Atomic write to prevent file corruption during updates
   */
  private async atomicWrite(filePath: string, content: string): Promise<void> {
    const tempPath = `${filePath}.claudecat.tmp`;
    
    try {
      // Write to temporary file first
      writeFileSync(tempPath, content, 'utf8');
      
      // Atomic move to final location
      renameSync(tempPath, filePath);
    } catch (error) {
      // Clean up temp file if it exists
      try {
        if (existsSync(tempPath)) {
          writeFileSync(tempPath, '', 'utf8'); // Clear temp file
        }
      } catch {
        // Ignore cleanup errors
      }
      throw error;
    }
  }

  /**
   * Escape special regex characters
   */
  private escapeRegex(string: string): string {
    return string.replace(/[.*+?^${}()|[\\]\\\\]/g, '\\\\$&');
  }
}