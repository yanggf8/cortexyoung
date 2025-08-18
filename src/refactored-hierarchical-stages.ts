// Refactored hierarchical stage tracking using composition architecture
import { CompositeLogger, UnifiedLogger } from './interfaces/logging-interface';
import { STAGE_CONSTANTS, StageId } from './constants/stage-constants';

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
  currentStageId?: string;
  stages: Map<string, HierarchicalStage>;
  startTime: number;
  endTime?: number;
  totalDuration?: number;
}

export class RefactoredHierarchicalStageTracker {
  private progress: HierarchicalProgress;
  private logger: UnifiedLogger;
  private currentStageId?: string;

  constructor() {
    this.logger = new CompositeLogger();
    this.progress = {
      overallStatus: 'initializing',
      stages: new Map(),
      startTime: Date.now()
    };

    this.initializeStages();
  }

  private initializeStages(): void {
    // Initialize all stages using centralized constants
    const stageConfigs = [
      {
        id: STAGE_CONSTANTS.STAGE_IDS.INITIALIZATION,
        name: STAGE_CONSTANTS.STAGE_NAMES.stage_1,
        description: 'System initialization and configuration loading',
        substeps: [
          { id: '1.1', name: 'Environment Setup', description: 'Load configuration, validate settings' },
          { id: '1.2', name: 'Storage Initialization', description: 'Vector store setup, persistence layer' },
          { id: '1.3', name: 'AI Model Loading', description: 'BGE-small-en-v1.5 initialization and readiness' }
        ]
      },
      {
        id: STAGE_CONSTANTS.STAGE_IDS.CODE_INTELLIGENCE,
        name: STAGE_CONSTANTS.STAGE_NAMES.stage_2,
        description: 'Repository analysis and semantic indexing',
        substeps: [
          { id: '2.1', name: 'Repository Analysis', description: 'File discovery, git scanning, content classification' },
          { id: '2.2', name: 'Semantic Processing', description: 'Code chunking, embedding generation, vector storage' },
          { id: '2.3', name: 'Relationship Analysis', description: 'Dependency mapping, symbol extraction, graph building' }
        ]
      },
      {
        id: STAGE_CONSTANTS.STAGE_IDS.SERVER_ACTIVATION,
        name: STAGE_CONSTANTS.STAGE_NAMES.stage_3,
        description: 'MCP server startup and service activation',
        substeps: [
          { id: '3.1', name: 'MCP Server Startup', description: 'HTTP transport, endpoint registration, service availability' }
        ]
      }
    ];

    for (const config of stageConfigs) {
      const stage: HierarchicalStage = {
        id: config.id,
        name: config.name,
        description: config.description,
        status: 'pending',
        substeps: config.substeps.map(substep => ({
          ...substep,
          status: 'pending' as const
        }))
      };
      this.progress.stages.set(config.id, stage);
    }
  }

  startStage(stageId: StageId): void {
    const stage = this.progress.stages.get(stageId);
    if (!stage) {
      this.logger.warn(`Unknown stage: ${stageId}`);
      return;
    }

    // Update state FIRST
    stage.status = 'in_progress';
    stage.startTime = Date.now();
    this.currentStageId = stageId;
    this.progress.currentStageId = stageId;

    // Log ONCE using unified logger
    const stageNumber = this.getStageNumber(stageId);
    this.logger.startStage(stageNumber, stage.name);
  }

  startSubstep(stageId: StageId, substepId: string, details?: string): void {
    const stage = this.progress.stages.get(stageId);
    if (!stage) {
      this.logger.warn(`Unknown stage: ${stageId}`);
      return;
    }

    const substep = stage.substeps.find(s => s.id === substepId);
    if (!substep) {
      this.logger.warn(`Unknown substep: ${substepId} in stage ${stageId}`);
      return;
    }

    // Update state FIRST
    substep.status = 'in_progress';
    substep.startTime = Date.now();
    substep.details = details;
    stage.currentSubstep = substepId;

    // Log ONCE using unified logger
    this.logger.startSubstep(substepId, substep.name, details || substep.description);
  }

  completeSubstep(stageId: StageId, substepId: string, result?: string): void {
    const stage = this.progress.stages.get(stageId);
    if (!stage) {
      this.logger.warn(`Unknown stage: ${stageId}`);
      return;
    }

    const substep = stage.substeps.find(s => s.id === substepId);
    if (!substep) {
      this.logger.warn(`Unknown substep: ${substepId} in stage ${stageId}`);
      return;
    }

    // Update state FIRST
    substep.status = 'completed';
    substep.endTime = Date.now();
    substep.duration = substep.endTime - (substep.startTime || substep.endTime);

    // Log ONCE using unified logger
    this.logger.completeSubstep(substepId, substep.name, result);
  }

  completeStage(stageId: StageId, result?: string): void {
    const stage = this.progress.stages.get(stageId);
    if (!stage) {
      this.logger.warn(`Unknown stage: ${stageId}`);
      return;
    }

    // Update state FIRST
    stage.status = 'completed';
    stage.endTime = Date.now();
    stage.duration = stage.endTime - (stage.startTime || stage.endTime);

    // Log ONCE using unified logger
    const stageNumber = this.getStageNumber(stageId);
    const duration = this.formatDuration(stage.duration);
    this.logger.completeStage(stageNumber, stage.name, duration);

    // Check if all stages are complete
    const allStagesComplete = Array.from(this.progress.stages.values())
      .every(s => s.status === 'completed');
    
    if (allStagesComplete) {
      this.progress.overallStatus = 'ready';
      this.progress.endTime = Date.now();
      this.progress.totalDuration = this.progress.endTime - this.progress.startTime;
    }
  }

  failStage(stageId: StageId, error: string): void {
    const stage = this.progress.stages.get(stageId);
    if (!stage) {
      this.logger.warn(`Unknown stage: ${stageId}`);
      return;
    }

    // Update state FIRST
    stage.status = 'failed';
    stage.endTime = Date.now();
    stage.duration = stage.endTime - (stage.startTime || stage.endTime);
    this.progress.overallStatus = 'failed';

    // Log ONCE using unified logger
    const stageNumber = this.getStageNumber(stageId);
    const duration = this.formatDuration(stage.duration);
    this.logger.failStage(stageNumber, stage.name, error, duration);
  }

  failSubstep(stageId: StageId, substepId: string, error: string): void {
    const stage = this.progress.stages.get(stageId);
    if (!stage) {
      this.logger.warn(`Unknown stage: ${stageId}`);
      return;
    }

    const substep = stage.substeps.find(s => s.id === substepId);
    if (!substep) {
      this.logger.warn(`Unknown substep: ${substepId} in stage ${stageId}`);
      return;
    }

    // Update state FIRST
    substep.status = 'failed';
    substep.endTime = Date.now();
    substep.duration = substep.endTime - (substep.startTime || substep.endTime);
    substep.error = error;

    // Log ONCE using unified logger
    this.logger.failSubstep(substepId, substep.name, error);
  }

  private getStageNumber(stageId: string): number {
    switch (stageId) {
      case STAGE_CONSTANTS.STAGE_IDS.INITIALIZATION: return 1;
      case STAGE_CONSTANTS.STAGE_IDS.CODE_INTELLIGENCE: return 2;
      case STAGE_CONSTANTS.STAGE_IDS.SERVER_ACTIVATION: return 3;
      default: return 0;
    }
  }

  private formatDuration(milliseconds: number): string {
    if (milliseconds < 1000) {
      return `${milliseconds}ms`;
    } else if (milliseconds < 60000) {
      return `${(milliseconds / 1000).toFixed(1)}s`;
    } else {
      const minutes = Math.floor(milliseconds / 60000);
      const seconds = ((milliseconds % 60000) / 1000).toFixed(1);
      return `${minutes}m ${seconds}s`;
    }
  }

  // Public API methods
  getProgress(): HierarchicalProgress {
    return { ...this.progress };
  }

  getCurrentStage(): HierarchicalStage | undefined {
    return this.currentStageId ? this.progress.stages.get(this.currentStageId) : undefined;
  }

  getSummary(): string {
    const completed = Array.from(this.progress.stages.values()).filter(s => s.status === 'completed').length;
    let summary = `${completed}/${STAGE_CONSTANTS.TOTAL_STAGES} stages completed`;

    const currentStage = this.getCurrentStage();
    if (currentStage) {
      summary += ` - Stage ${this.getStageNumber(currentStage.id)}/${STAGE_CONSTANTS.TOTAL_STAGES}: ${currentStage.name}`;
    }

    return summary;
  }

  // Legacy compatibility - delegate to unified logger
  logInfo(message: string): void {
    this.logger.info(message);
  }

  logWarning(message: string): void {
    this.logger.warn(message);
  }

  logError(message: string): void {
    this.logger.error(message);
  }
}