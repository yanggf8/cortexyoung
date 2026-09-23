import { EventEmitter } from 'events';
import { EnhancedProjectDetector } from './project-detector.js';
import { CLAUDEMdMaintainer } from './claude-md-maintainer.js';
import { ContextWatcher } from './context-watcher.js';
import type { ProjectContextInfo } from '../types/patterns.js';

export interface ProactiveContextEngineEvents {
  'context-updated': [ProjectContextInfo];
  'context-error': [Error];
  'startup-complete': [];
}

export class ProactiveContextEngine extends EventEmitter<ProactiveContextEngineEvents> {
  private detector: EnhancedProjectDetector;
  private maintainer: CLAUDEMdMaintainer;
  private watcher: ContextWatcher;
  private projectRoot: string;
  private isInitialized = false;

  constructor(projectRoot: string = process.cwd()) {
    super();
    this.projectRoot = projectRoot;
    this.detector = new EnhancedProjectDetector(projectRoot);
    this.maintainer = new CLAUDEMdMaintainer(projectRoot);
    this.watcher = new ContextWatcher(this.detector, this.maintainer, projectRoot);

    // Forward watcher events
    this.watcher.on('context-updated', () => {
      const context = this.detector.detectCurrentContext();
      this.emit('context-updated', context);
    });
    
    this.watcher.on('error', (error) => {
      this.emit('context-error', error);
    });
  }

  /**
   * Initialize the proactive context engine
   */
  async initialize(): Promise<void> {
    if (this.isInitialized) {
      console.log('✅ Proactive Context Engine already initialized');
      return;
    }

    try {
      console.log('🚀 Initializing Proactive Context Engine...');
      
      // 1. Detect current project context
      console.log('🔍 Detecting project implementation patterns...');
      const initialContext = this.detector.detectCurrentContext();
      
      // 2. Update CLAUDE.md with detected patterns
      console.log('📝 Updating CLAUDE.md with detected patterns...');
      await this.maintainer.updateProjectContext(initialContext);
      
      // 3. Start file watching for real-time updates
      console.log('👀 Starting file monitoring...');
      this.watcher.startWatching();
      
      this.isInitialized = true;
      console.log('✅ Proactive Context Engine initialized successfully');
      
      this.emit('startup-complete');
      this.emit('context-updated', initialContext);
      
    } catch (error) {
      console.error('❌ Failed to initialize Proactive Context Engine:', error);
      this.emit('context-error', error instanceof Error ? error : new Error(String(error)));
      throw error;
    }
  }

  /**
   * Get current project context
   */
  getCurrentContext(): ProjectContextInfo {
    return this.detector.detectCurrentContext();
  }

  /**
   * Force immediate context update
   */
  async forceUpdate(): Promise<ProjectContextInfo> {
    try {
      const context = this.detector.detectCurrentContext();
      await this.maintainer.updateProjectContext(context);
      this.emit('context-updated', context);
      return context;
    } catch (error) {
      const err = error instanceof Error ? error : new Error(String(error));
      this.emit('context-error', err);
      throw err;
    }
  }

  /**
   * Get engine status
   */
  getStatus() {
    const watcherStatus = this.watcher.getStatus();
    const watcherStats = this.watcher.getStats();
    
    return {
      initialized: this.isInitialized,
      projectRoot: this.projectRoot,
      watcher: {
        ...watcherStatus,
        ...watcherStats
      }
    };
  }

  /**
   * Shutdown the engine
   */
  async shutdown(): Promise<void> {
    console.log('🛑 Shutting down Proactive Context Engine...');
    
    this.watcher.stopWatching();
    this.removeAllListeners();
    this.isInitialized = false;
    
    console.log('✅ Proactive Context Engine shut down');
  }

  /**
   * Get implementation patterns summary for MCP responses
   */
  getImplementationPatternsSummary(): {
    auth: string;
    api: string;
    errors: string;
    confidence: string;
  } {
    const context = this.getCurrentContext();
    const impl = context.implementationPatterns;
    
    return {
      auth: `${impl.authentication.userProperty} | ${impl.authentication.tokenLocation} | ${impl.authentication.errorResponse.format}`,
      api: `${impl.apiResponses.successFormat} | ${impl.apiResponses.errorFormat} | ${impl.apiResponses.wrapperPattern}`,
      errors: `${impl.errorHandling.catchPattern} | ${impl.errorHandling.errorStructure} | ${impl.errorHandling.propagationStyle}`,
      confidence: `Auth: ${impl.authentication.confidence}%, API: ${impl.apiResponses.confidence}%, Errors: ${impl.errorHandling.confidence}%`
    };
  }

  /**
   * Get critical guardrails for MCP responses
   */
  getCriticalGuardrails(): string[] {
    const context = this.getCurrentContext();
    const impl = context.implementationPatterns;
    const guardrails: string[] = [];

    // Authentication guardrails
    if (impl.authentication.tokenLocation === 'httpOnly cookie' && impl.authentication.confidence >= 60) {
      guardrails.push('NEVER use localStorage for tokens - Project uses httpOnly cookies');
    }
    if (impl.authentication.userProperty !== 'Unknown' && impl.authentication.confidence >= 60) {
      guardrails.push(`ALWAYS use ${impl.authentication.userProperty} for authenticated user data`);
    }

    // API response guardrails  
    if (impl.apiResponses.successFormat !== 'Unknown' && impl.apiResponses.confidence >= 60) {
      if (impl.apiResponses.successFormat === 'bare object') {
        guardrails.push('Use bare object responses - No wrapper format detected');
      } else {
        guardrails.push(`ALWAYS wrap API responses in ${impl.apiResponses.successFormat} format`);
      }
    }

    // Error handling guardrails
    if (impl.errorHandling.catchPattern !== 'Unknown' && impl.errorHandling.confidence >= 60) {
      guardrails.push(`Follow ${impl.errorHandling.catchPattern} error handling pattern`);
    }

    return guardrails;
  }
}