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
    
    // Get step number for display
    const stageArray = Array.from(this.stages.values());
    const currentStepIndex = stageArray.findIndex(s => s.id === stageId) + 1;
    const totalSteps = stageArray.length;
    
    console.log(`🚀 [Step ${currentStepIndex}/${totalSteps}] ${stage.name}: ${stage.description}`);
    if (details) console.log(`   Details: ${details}`);
  }

  updateStageProgress(stageId: string, progress: number, details?: string): void {
    const stage = this.stages.get(stageId);
    if (!stage) return;

    stage.progress = Math.min(100, Math.max(0, progress));
    if (details) stage.details = details;

    // Get step number for display
    const stageArray = Array.from(this.stages.values());
    const currentStepIndex = stageArray.findIndex(s => s.id === stageId) + 1;
    const totalSteps = stageArray.length;
    
    // Calculate elapsed time for this stage
    const elapsedMs = stage.startTime ? Date.now() - stage.startTime : 0;
    const elapsedSeconds = (elapsedMs / 1000).toFixed(1);

    console.log(`📊 [Step ${currentStepIndex}/${totalSteps}] ${stage.name}: ${progress.toFixed(1)}% (${elapsedSeconds}s)${details ? ` - ${details}` : ''}`);
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

    // Get step number for display
    const stageArray = Array.from(this.stages.values());
    const currentStepIndex = stageArray.findIndex(s => s.id === stageId) + 1;
    const totalSteps = stageArray.length;
    
    // Format duration nicely
    const durationMs = stage.duration || 0;
    const durationFormatted = durationMs >= 1000 
      ? `${(durationMs / 1000).toFixed(1)}s` 
      : `${durationMs}ms`;

    console.log(`✅ [Step ${currentStepIndex}/${totalSteps}] ${stage.name} completed (${durationFormatted})`);
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

    // Get step number for display
    const stageArray = Array.from(this.stages.values());
    const currentStepIndex = stageArray.findIndex(s => s.id === stageId) + 1;
    const totalSteps = stageArray.length;
    
    // Format duration nicely
    const durationMs = stage.duration || 0;
    const durationFormatted = durationMs >= 1000 
      ? `${(durationMs / 1000).toFixed(1)}s` 
      : `${durationMs}ms`;

    console.error(`❌ [Step ${currentStepIndex}/${totalSteps}] ${stage.name} failed (${durationFormatted}): ${error}`);
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
    summary += `(Step ${stats.completed + 1}/${stats.total})`;
    
    if (currentStage) {
      summary += ` - ${currentStage.name}`;
      if (currentStage.progress !== undefined && currentStage.progress > 0) {
        summary += ` ${currentStage.progress.toFixed(1)}%`;
      }
      
      // Add elapsed time for current stage
      if (currentStage.startTime) {
        const elapsedMs = Date.now() - currentStage.startTime;
        const elapsedFormatted = elapsedMs >= 1000 
          ? `${(elapsedMs / 1000).toFixed(1)}s` 
          : `${elapsedMs}ms`;
        summary += ` (${elapsedFormatted})`;
      }
    }

    if (progress.estimatedTimeRemaining) {
      const eta = Math.round(progress.estimatedTimeRemaining / 1000);
      summary += ` - ETA: ${eta}s`;
    }

    return summary;
  }

  // Utility functions for endpoints
  getCurrentStepIndex(): number {
    const currentStage = this.getCurrentStage();
    const progress = this.getProgress();
    const stats = this.getStageStats();
    
    return currentStage ? 
      progress.stages.findIndex(s => s.id === currentStage.id) + 1 : 
      stats.completed + 1;
  }

  getCurrentStepString(): string {
    const stats = this.getStageStats();
    const currentStepIndex = this.getCurrentStepIndex();
    return `${currentStepIndex}/${stats.total}`;
  }

  formatDuration(durationMs: number): string {
    return durationMs >= 1000 
      ? `${(durationMs / 1000).toFixed(1)}s` 
      : `${durationMs}ms`;
  }

  getCurrentStageElapsed(): string | null {
    const currentStage = this.getCurrentStage();
    if (!currentStage?.startTime) return null;
    
    const elapsedMs = Date.now() - currentStage.startTime;
    return this.formatDuration(elapsedMs);
  }

  // Enhanced progress with step numbers and formatted timing
  getEnhancedProgress() {
    const progress = this.getProgress();
    const stats = this.getStageStats();
    const currentStepIndex = this.getCurrentStepIndex();
    
    const enhancedStages = progress.stages.map((stage, index) => {
      const stepNumber = index + 1;
      let durationFormatted: string | null = null;
      
      if (stage.duration) {
        durationFormatted = this.formatDuration(stage.duration);
      } else if (stage.startTime && stage.status === 'in_progress') {
        const elapsedMs = Date.now() - stage.startTime;
        durationFormatted = this.formatDuration(elapsedMs);
      }
      
      return {
        ...stage,
        step: `${stepNumber}/${stats.total}`,
        durationFormatted
      };
    });
    
    return {
      ...progress,
      stages: enhancedStages,
      currentStep: `${currentStepIndex}/${stats.total}`,
      summary: this.getProgressSummary()
    };
  }

  // Health endpoint data
  getHealthData() {
    const progress = this.getProgress();
    const currentStage = this.getCurrentStage();
    const isReady = this.isReady();
    const stats = this.getStageStats();
    
    // Determine health status
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
    
    // Add startup progress info if not fully ready
    if (!isReady) {
      response.startup = {
        stage: currentStage?.name || 'Unknown',
        step: this.getCurrentStepString(),
        progress: Math.round(progress.overallProgress),
        stageProgress: currentStage?.progress ? Math.round(currentStage.progress) : 0,
        elapsed: this.getCurrentStageElapsed(),
        eta: progress.estimatedTimeRemaining ? Math.round(progress.estimatedTimeRemaining / 1000) : null,
        completed: stats.completed,
        total: stats.total,
        details: currentStage?.details,
        summary: this.getProgressSummary()
      };
    }
    
    // Add any failed stages
    if (stats.failed > 0) {
      const failedStages = progress.stages
        .filter(stage => stage.status === 'failed')
        .map(stage => ({ name: stage.name, error: stage.error }));
      response.errors = failedStages;
    }
    
    return response;
  }

  // Status endpoint data
  getStatusData() {
    const progress = this.getProgress();
    const currentStage = this.getCurrentStage();
    const stats = this.getStageStats();
    
    return {
      status: progress.overallStatus,
      ready: this.isReady(),
      progress: progress.overallProgress,
      currentStage: currentStage?.name,
      step: this.getCurrentStepString(),
      stageProgress: currentStage?.progress,
      elapsed: this.getCurrentStageElapsed(),
      stages: `${stats.completed}/${stats.total}`,
      eta: progress.estimatedTimeRemaining ? Math.round(progress.estimatedTimeRemaining / 1000) : null,
      summary: this.getProgressSummary(),
      server: 'cortex-mcp-server',
      timestamp: Date.now()
    };
  }

  // Progress endpoint data
  getProgressData() {
    const enhancedProgress = this.getEnhancedProgress();
    
    return {
      ...enhancedProgress,
      server: 'cortex-mcp-server',
      version: '2.1.0',
      timestamp: Date.now()
    };
  }
}