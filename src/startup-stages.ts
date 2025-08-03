// Startup stage tracking for MCP server initialization

export interface StartupStage {
  id: string;
  name: string;
  description: string;
  status: 'pending' | 'in_progress' | 'completed' | 'failed';
  startTime?: number;
  endTime?: number;
  duration?: number;
  progress?: number; // 0-100 percentage
  details?: string;
  error?: string;
}

export interface StartupProgress {
  overallStatus: 'initializing' | 'indexing' | 'ready' | 'failed';
  overallProgress: number; // 0-100 percentage
  currentStage?: string;
  stages: StartupStage[];
  totalDuration?: number;
  estimatedTimeRemaining?: number;
}

export class StartupStageTracker {
  private stages: Map<string, StartupStage> = new Map();
  private currentStageId?: string;
  private startTime: number;

  constructor() {
    this.startTime = Date.now();
    this.initializeStages();
  }

  private initializeStages(): void {
    const stageDefinitions: Omit<StartupStage, 'status' | 'startTime' | 'endTime' | 'duration' | 'progress'>[] = [
      {
        id: 'server_init',
        name: 'Server Initialization',
        description: 'Initializing MCP server and basic configuration'
      },
      {
        id: 'cache_check',
        name: 'Cache Detection',
        description: 'Checking for existing embeddings cache'
      },
      {
        id: 'model_load',
        name: 'AI Model Loading',
        description: 'Loading BGE-small-en-v1.5 embedding model'
      },
      {
        id: 'file_scan',
        name: 'File Discovery',
        description: 'Scanning repository for code files'
      },
      {
        id: 'delta_analysis',
        name: 'Change Detection',
        description: 'Analyzing which files have changed since last index'
      },
      {
        id: 'code_chunking',
        name: 'Code Chunking',
        description: 'Breaking code files into semantic chunks'
      },
      {
        id: 'embedding_generation',
        name: 'Embedding Generation',
        description: 'Generating vector embeddings for code chunks'
      },
      {
        id: 'relationship_analysis',
        name: 'Relationship Analysis',
        description: 'Building relationship graph between code elements'
      },
      {
        id: 'vector_storage',
        name: 'Vector Storage',
        description: 'Storing embeddings and relationships to cache'
      },
      {
        id: 'mcp_ready',
        name: 'MCP Ready',
        description: 'MCP server ready to accept requests'
      }
    ];

    stageDefinitions.forEach(stageDef => {
      this.stages.set(stageDef.id, {
        ...stageDef,
        status: 'pending',
        progress: 0
      });
    });
  }

  startStage(stageId: string, details?: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) {
      console.warn(`Unknown stage: ${stageId}`);
      return;
    }

    // Complete previous stage if it was in progress
    if (this.currentStageId && this.currentStageId !== stageId) {
      this.completeStage(this.currentStageId);
    }

    stage.status = 'in_progress';
    stage.startTime = Date.now();
    stage.progress = 0;
    if (details) stage.details = details;
    
    this.currentStageId = stageId;
    
    console.log(`🚀 [Stage] ${stage.name}: ${stage.description}`);
    if (details) console.log(`   Details: ${details}`);
  }

  updateStageProgress(stageId: string, progress: number, details?: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    stage.progress = Math.min(100, Math.max(0, progress));
    if (details) stage.details = details;

    console.log(`📊 [Progress] ${stage.name}: ${progress.toFixed(1)}%${details ? ` - ${details}` : ''}`);
  }

  completeStage(stageId: string, details?: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    stage.status = 'completed';
    stage.endTime = Date.now();
    stage.progress = 100;
    if (stage.startTime) {
      stage.duration = stage.endTime - stage.startTime;
    }
    if (details) stage.details = details;

    console.log(`✅ [Complete] ${stage.name} (${stage.duration}ms)`);
  }

  failStage(stageId: string, error: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    stage.status = 'failed';
    stage.endTime = Date.now();
    stage.error = error;
    if (stage.startTime) {
      stage.duration = stage.endTime - stage.startTime;
    }

    console.error(`❌ [Failed] ${stage.name}: ${error}`);
  }

  getProgress(): StartupProgress {
    const stageArray = Array.from(this.stages.values());
    const completedStages = stageArray.filter(s => s.status === 'completed').length;
    const totalStages = stageArray.length;
    const overallProgress = (completedStages / totalStages) * 100;

    // Determine overall status
    let overallStatus: StartupProgress['overallStatus'] = 'initializing';
    if (stageArray.some(s => s.status === 'failed')) {
      overallStatus = 'failed';
    } else if (completedStages === totalStages) {
      overallStatus = 'ready';
    } else if (this.currentStageId === 'embedding_generation' || this.currentStageId === 'code_chunking') {
      overallStatus = 'indexing';
    }

    // Calculate estimated time remaining
    let estimatedTimeRemaining: number | undefined;
    const inProgressStages = stageArray.filter(s => s.status === 'in_progress');
    if (inProgressStages.length > 0 && completedStages > 0) {
      const avgStageTime = stageArray
        .filter(s => s.duration)
        .reduce((sum, s) => sum + (s.duration || 0), 0) / completedStages;
      const remainingStages = totalStages - completedStages;
      estimatedTimeRemaining = remainingStages * avgStageTime;
    }

    return {
      overallStatus,
      overallProgress,
      currentStage: this.currentStageId,
      stages: stageArray,
      totalDuration: Date.now() - this.startTime,
      estimatedTimeRemaining
    };
  }

  getCurrentStage(): StartupStage | undefined {
    return this.currentStageId ? this.stages.get(this.currentStageId) : undefined;
  }

  isReady(): boolean {
    return Array.from(this.stages.values()).every(stage => stage.status === 'completed');
  }

  getStageStats(): { completed: number; total: number; failed: number } {
    const stageArray = Array.from(this.stages.values());
    return {
      completed: stageArray.filter(s => s.status === 'completed').length,
      total: stageArray.length,
      failed: stageArray.filter(s => s.status === 'failed').length
    };
  }

  // Public method to get formatted progress for logging
  getProgressSummary(): string {
    const progress = this.getProgress();
    const stats = this.getStageStats();
    const currentStage = this.getCurrentStage();
    
    let summary = `[${progress.overallStatus.toUpperCase()}] ${progress.overallProgress.toFixed(1)}% `;
    summary += `(${stats.completed}/${stats.total} stages)`;
    
    if (currentStage) {
      summary += ` - ${currentStage.name}`;
      if (currentStage.progress !== undefined && currentStage.progress > 0) {
        summary += ` ${currentStage.progress.toFixed(1)}%`;
      }
    }

    if (progress.estimatedTimeRemaining) {
      const eta = Math.round(progress.estimatedTimeRemaining / 1000);
      summary += ` - ETA: ${eta}s`;
    }

    return summary;
  }
}