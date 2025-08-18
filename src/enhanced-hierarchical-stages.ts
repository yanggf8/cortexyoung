// Enhanced hierarchical stage tracking using new console logger
import { conditionalLogger } from './utils/console-logger';
import { HierarchicalStageTracker } from './hierarchical-stages';

export class EnhancedHierarchicalStageTracker extends HierarchicalStageTracker {
  constructor(logger?: any) {
    super(logger);
  }

  // Override the key methods to use new console logger
  startStage(stageId: string): void {
    // Call parent to maintain state
    super.startStage(stageId);
    
    // Override the logging part with new console logger
    const stageNumber = this.getEnhancedStageNumber(stageId);
    const stageName = this.getStageName(stageId);
    
    // Use new console logger for stage start
    conditionalLogger.stage.start(stageNumber, 3, stageName);
  }

  startSubstep(stageId: string, substepId: string, details?: string): void {
    // Call parent to maintain state
    super.startSubstep(stageId, substepId, details);
    
    // Override the logging part with new console logger  
    const substepName = this.getSubstepName(stageId, substepId);
    conditionalLogger.step.start(substepId, substepName, details);
  }

  completeSubstep(stageId: string, substepId: string, details?: string): void {
    // Call parent to maintain state
    super.completeSubstep(stageId, substepId, details);
    
    // Override the logging part with new console logger
    conditionalLogger.step.complete(details);
  }

  completeStage(stageId: string): void {
    // Call parent to maintain state
    super.completeStage(stageId);
    
    // Override the logging part with new console logger
    conditionalLogger.stage.complete('Success');
  }

  failStage(stageId: string, error: string): void {
    // Call parent to maintain state
    super.failStage(stageId, error);
    
    // Override the logging part with new console logger
    conditionalLogger.stage.fail(error);
  }

  failSubstep(stageId: string, substepId: string, error: string): void {
    // Call parent to maintain state
    super.failSubstep(stageId, substepId, error);
    
    // Override the logging part with new console logger
    conditionalLogger.step.fail(error);
  }

  logStartupSummary(): void {
    // Call parent to maintain state
    super.logStartupSummary();
    
    // Override the logging part with new console logger
    const totalDuration = Date.now() - (this as any).startTime;
    conditionalLogger.ready('Cortex MCP Server Ready!', {
      metadata: {
        totalDuration: this.formatEnhancedDuration(totalDuration),
        stages: '3/3 completed',
        status: 'operational'
      }
    });
  }

  private getEnhancedStageNumber(stageId: string): number {
    switch (stageId) {
      case 'stage_1': return 1;
      case 'stage_2': return 2; 
      case 'stage_3': return 3;
      default: return 0;
    }
  }

  private getStageName(stageId: string): string {
    switch (stageId) {
      case 'stage_1': return 'Initialization & Pre-flight Checks';
      case 'stage_2': return 'Code Intelligence Indexing';
      case 'stage_3': return 'Server Activation';
      default: return 'Unknown Stage';
    }
  }

  private getSubstepName(stageId: string, substepId: string): string {
    const substepMap: Record<string, Record<string, string>> = {
      'stage_1': {
        '1.1': 'Server Initialization',
        '1.2': 'Cache & Storage Health Check',
        '1.3': 'AI Model Loading'
      },
      'stage_2': {
        '2.1': 'Repository Analysis',
        '2.2': 'Embedding Generation',
        '2.3': 'Relationship Analysis',
        '2.4': 'Vector Storage Commit'
      },
      'stage_3': {
        '3.1': 'MCP Server Startup'
      }
    };
    
    return substepMap[stageId]?.[substepId] || 'Unknown Step';
  }

  private formatEnhancedDuration(durationMs: number): string {
    return durationMs >= 1000 
      ? `${(durationMs / 1000).toFixed(1)}s` 
      : `${durationMs}ms`;
  }
}