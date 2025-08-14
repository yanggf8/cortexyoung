// Simple 3-stage tracking for MCP server initialization
import { timestampedLog, warn as timestampedWarn, error as timestampedError } from './logging-utils';
// Uses clear break line delimiters for stage entry and exit

export interface HierarchicalSubstep {
  id: string;
  name: string;
  description: string;
  status: 'pending' | 'in_progress' | 'completed' | 'failed';
  startTime?: number;
  endTime?: number;
  duration?: number;
  details?: string;
  error?: string;
}

export interface HierarchicalStage {
  id: string;
  name: string;
  description: string;
  status: 'pending' | 'in_progress' | 'completed' | 'failed';
  startTime?: number;
  endTime?: number;
  duration?: number;
  substeps: HierarchicalSubstep[];
  currentSubstep?: string;
}

export interface HierarchicalProgress {
  overallStatus: 'initializing' | 'indexing' | 'ready' | 'failed';
  overallProgress: number; // 0-100 percentage
  currentStage?: string;
  currentSubstep?: string;
  stages: HierarchicalStage[];
  totalDuration?: number;
  estimatedTimeRemaining?: number;
}

export class HierarchicalStageTracker {
  private stages: Map<string, HierarchicalStage> = new Map();
  private currentStageId?: string;
  private currentSubstepId?: string;
  private startTime: number;
  private logger?: any;

  constructor(logger?: any) {
    this.startTime = Date.now();
    this.logger = logger;
    this.initializeStages();
  }

  private initializeStages(): void {
    const stageDefinitions: Omit<HierarchicalStage, 'status' | 'startTime' | 'endTime' | 'duration' | 'substeps' | 'currentSubstep'>[] = [
      {
        id: 'stage_1',
        name: 'Initialization & Pre-flight Checks',
        description: 'Setting up server components and validating system state'
      },
      {
        id: 'stage_2', 
        name: 'Code Intelligence Indexing',
        description: 'Analyzing repository and building semantic intelligence'
      },
      {
        id: 'stage_3',
        name: 'Server Activation',
        description: 'Starting MCP server and accepting requests'
      }
    ];

    // Define substeps for each stage
    const substepDefinitions: Record<string, Omit<HierarchicalSubstep, 'status' | 'startTime' | 'endTime' | 'duration'>[]> = {
      'stage_1': [
        {
          id: '1.1',
          name: 'Server Initialization',
          description: 'Logger setup, repository validation, MCP server setup'
        },
        {
          id: '1.2', 
          name: 'Cache & Storage Health Check',
          description: 'Storage comparison, cache loading, auto-sync, health validation'
        },
        {
          id: '1.3',
          name: 'AI Model Loading', 
          description: 'BGE-small-en-v1.5 initialization and readiness confirmation'
        }
      ],
      'stage_2': [
        {
          id: '2.1',
          name: 'Repository Analysis',
          description: 'File discovery, delta analysis, change categorization'
        },
        {
          id: '2.2',
          name: 'Embedding Generation',
          description: 'Process pool setup, chunk processing, resource monitoring'
        },
        {
          id: '2.3', 
          name: 'Relationship Analysis',
          description: 'Dependency mapping, symbol extraction, relationship graph building'
        },
        {
          id: '2.4',
          name: 'Vector Storage Commit',
          description: 'Database updates, storage persistence, synchronization'
        }
      ],
      'stage_3': [
        {
          id: '3.1',
          name: 'MCP Server Startup',
          description: 'HTTP transport, endpoint registration, service availability'
        }
      ]
    };

    // Initialize stages with their substeps
    stageDefinitions.forEach(stageDef => {
      const substeps: HierarchicalSubstep[] = (substepDefinitions[stageDef.id] || []).map(substepDef => ({
        ...substepDef,
        status: 'pending'
      }));

      this.stages.set(stageDef.id, {
        ...stageDef,
        status: 'pending',
        substeps,
        currentSubstep: undefined
      });
    });
  }

  startStage(stageId: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) {
      this.logWarning(`Unknown stage: ${stageId}`);
      return;
    }

    // Complete previous stage if it was in progress
    if (this.currentStageId && this.currentStageId !== stageId) {
      this.completeStage(this.currentStageId);
    }

    stage.status = 'in_progress';
    stage.startTime = Date.now();
    this.currentStageId = stageId;
    
    const stageNumber = this.getStageNumber(stageId);
    
    // Stage entry with break line delimiter
    this.logInfo('==========================================');
    this.logInfo(`🚀 STAGE ${stageNumber}/3: ${stage.name.toUpperCase()}`);
    this.logInfo('==========================================');
  }

  startSubstep(stageId: string, substepId: string, details?: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) {
      this.logWarning(`Unknown stage: ${stageId}`);
      return;
    }

    const substep = stage.substeps.find(s => s.id === substepId);
    if (!substep) {
      this.logWarning(`Unknown substep: ${substepId} in stage ${stageId}`);
      return;
    }

    // Complete previous substep if it was in progress
    if (this.currentSubstepId && stage.currentSubstep) {
      this.completeSubstep(stageId, stage.currentSubstep);
    }

    substep.status = 'in_progress';
    substep.startTime = Date.now();
    if (details) substep.details = details;
    
    stage.currentSubstep = substepId;
    this.currentSubstepId = substepId;
    
    // Step entry with break line delimiter
    this.logInfo('------------------------------------------');
    this.logInfo(`⚡ STEP ${substepId}: ${substep.name}`);
    if (details) {
      this.logInfo(`   Details: ${details}`);
    }
    this.logInfo('------------------------------------------');
  }

  completeSubstep(stageId: string, substepId: string, details?: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    const substep = stage.substeps.find(s => s.id === substepId);
    if (!substep || substep.status === 'completed') return;

    substep.status = 'completed';
    substep.endTime = Date.now();
    if (substep.startTime) {
      substep.duration = substep.endTime - substep.startTime;
    }
    if (details) substep.details = details;

    const durationFormatted = this.formatDuration(substep.duration || 0);
    
    // Step exit with break line delimiter
    this.logInfo('------------------------------------------');
    this.logInfo(`✅ STEP ${substepId} COMPLETED: ${substep.name}`);
    if (details) {
      this.logInfo(`   Result: ${details}`);
    }
    this.logInfo(`   Duration: ${durationFormatted}`);
    this.logInfo('------------------------------------------');
  }

  completeStage(stageId: string): void {
    const stage = this.stages.get(stageId);
    if (!stage || stage.status === 'completed') return;

    // Complete any remaining substep
    if (stage.currentSubstep) {
      this.completeSubstep(stageId, stage.currentSubstep);
    }

    stage.status = 'completed';
    stage.endTime = Date.now();
    if (stage.startTime) {
      stage.duration = stage.endTime - stage.startTime;
    }

    const stageNumber = this.getStageNumber(stageId);
    const durationFormatted = this.formatDuration(stage.duration || 0);
    
    // Stage exit with break line delimiter
    this.logInfo('==========================================');
    this.logInfo(`✅ STAGE ${stageNumber}/3 COMPLETED: ${stage.name.toUpperCase()}`);
    this.logInfo(`   Duration: ${durationFormatted}`);
    this.logInfo('==========================================');
  }

  failStage(stageId: string, error: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    stage.status = 'failed';
    stage.endTime = Date.now();
    if (stage.startTime) {
      stage.duration = stage.endTime - stage.startTime;
    }

    const stageNumber = this.getStageNumber(stageId);
    const durationFormatted = this.formatDuration(stage.duration || 0);
    
    // Stage failure with break line delimiter
    this.logError('==========================================');
    this.logError(`❌ STAGE ${stageNumber}/3 FAILED: ${stage.name.toUpperCase()}`);
    this.logError(`   Error: ${error}`);
    this.logError(`   Duration: ${durationFormatted}`);
    this.logError('==========================================');
  }

  failSubstep(stageId: string, substepId: string, error: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    const substep = stage.substeps.find(s => s.id === substepId);
    if (!substep) return;

    substep.status = 'failed';
    substep.endTime = Date.now();
    substep.error = error;
    if (substep.startTime) {
      substep.duration = substep.endTime - substep.startTime;
    }

    const durationFormatted = this.formatDuration(substep.duration || 0);
    
    // Step failure with break line delimiter
    this.logError('------------------------------------------');
    this.logError(`❌ STEP ${substepId} FAILED: ${substep.name}`);
    this.logError(`   Error: ${error}`);
    this.logError(`   Duration: ${durationFormatted}`);
    this.logError('------------------------------------------');
  }

  getProgress(): HierarchicalProgress {
    const stageArray = Array.from(this.stages.values());
    const completedStages = stageArray.filter(s => s.status === 'completed').length;
    const totalStages = stageArray.length;
    
    // Calculate overall progress including substeps
    let totalSubsteps = 0;
    let completedSubsteps = 0;
    
    stageArray.forEach(stage => {
      totalSubsteps += stage.substeps.length;
      completedSubsteps += stage.substeps.filter(s => s.status === 'completed').length;
    });
    
    const overallProgress = totalSubsteps > 0 ? (completedSubsteps / totalSubsteps) * 100 : 0;

    // Determine overall status
    let overallStatus: HierarchicalProgress['overallStatus'] = 'initializing';
    if (stageArray.some(s => s.status === 'failed')) {
      overallStatus = 'failed';
    } else if (completedStages === totalStages) {
      overallStatus = 'ready';
    } else if (this.currentStageId === 'stage_2') {
      overallStatus = 'indexing';
    }

    return {
      overallStatus,
      overallProgress,
      currentStage: this.currentStageId,
      currentSubstep: this.currentSubstepId,
      stages: stageArray,
      totalDuration: Date.now() - this.startTime
    };
  }

  logStartupSummary(): void {
    const totalDuration = Date.now() - this.startTime;
    const durationFormatted = this.formatDuration(totalDuration);
    
    // Final startup summary with break line delimiter
    this.logInfo('==========================================');
    this.logInfo('🎉 CORTEX MCP SERVER READY!');
    this.logInfo(`   Total startup duration: ${durationFormatted}`);
    this.logInfo('==========================================');
  }

  private getStageNumber(stageId: string): number {
    switch (stageId) {
      case 'stage_1': return 1;
      case 'stage_2': return 2; 
      case 'stage_3': return 3;
      default: return 0;
    }
  }

  private formatDuration(durationMs: number): string {
    return durationMs >= 1000 
      ? `${(durationMs / 1000).toFixed(1)}s` 
      : `${durationMs}ms`;
  }

  private logInfo(message: string): void {
    if (this.logger) {
      this.logger.info(message);
    } else {
      timestampedLog(`[INFO] ${message}`);
    }
  }

  private logWarning(message: string): void {
    if (this.logger) {
      this.logger.warn(message);
    } else {
      timestampedWarn(`[WARN] ${message}`);
    }
  }

  private logError(message: string): void {
    if (this.logger) {
      this.logger.error(message);
    } else {
      timestampedError(`[ERROR] ${message}`);
    }
  }

  // Backward compatibility methods for health/status endpoints
  isReady(): boolean {
    return Array.from(this.stages.values()).every(stage => stage.status === 'completed');
  }

  getCurrentStage(): HierarchicalStage | undefined {
    return this.currentStageId ? this.stages.get(this.currentStageId) : undefined;
  }

  getStageStats(): { completed: number; total: number; failed: number } {
    const stageArray = Array.from(this.stages.values());
    return {
      completed: stageArray.filter(s => s.status === 'completed').length,
      total: stageArray.length,
      failed: stageArray.filter(s => s.status === 'failed').length
    };
  }

  // Backward compatibility methods for existing code
  getProgressSummary(): string {
    const progress = this.getProgress();
    const currentStage = this.getCurrentStage();
    
    let summary = `[${progress.overallStatus.toUpperCase()}] ${progress.overallProgress.toFixed(1)}% `;
    if (currentStage) {
      summary += ` - Stage ${this.getStageNumber(currentStage.id)}/3: ${currentStage.name}`;
    }
    return summary;
  }

  getProgressData() {
    const progress = this.getProgress();
    const currentStage = this.getCurrentStage();
    
    return {
      ...progress,
      server: 'cortex-mcp-server',
      version: '2.1.0',
      timestamp: Date.now(),
      currentStageName: currentStage?.name,
      currentStageId: currentStage?.id
    };
  }

  getStatusData() {
    const progress = this.getProgress();
    const currentStage = this.getCurrentStage();
    const stats = this.getStageStats();
    
    return {
      status: progress.overallStatus,
      ready: this.isReady(),
      progress: progress.overallProgress,
      currentStage: currentStage?.name,
      step: `${stats.completed + 1}/${stats.total}`,
      completed: stats.completed,
      total: stats.total,
      summary: this.getProgressSummary(),
      server: 'cortex-mcp-server',
      timestamp: Date.now()
    };
  }

  // Health endpoint compatibility
  getHealthData() {
    const progress = this.getProgress();
    const currentStage = this.getCurrentStage();
    const isReady = this.isReady();
    const stats = this.getStageStats();
    
    let healthStatus: 'healthy' | 'starting' | 'indexing' | 'error';
    if (isReady) {
      healthStatus = 'healthy';
    } else if (progress.overallStatus === 'failed') {
      healthStatus = 'error';
    } else if (progress.overallStatus === 'indexing') {
      healthStatus = 'indexing';
    } else {
      healthStatus = 'starting';
    }
    
    const response: any = {
      status: healthStatus,
      server: 'cortex-mcp-server',
      version: '2.1.0',
      ready: isReady,
      timestamp: Date.now()
    };
    
    if (!isReady) {
      response.startup = {
        stage: currentStage?.name || 'Unknown',
        step: `${stats.completed + 1}/${stats.total}`,
        progress: Math.round(progress.overallProgress),
        completed: stats.completed,
        total: stats.total,
        summary: `Stage ${this.getStageNumber(this.currentStageId || 'stage_1')}/3: ${currentStage?.name || 'Unknown'}`
      };
    }
    
    return response;
  }
}