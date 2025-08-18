interface ActivityState {
  isActive: boolean;
  intensity: 'low' | 'medium' | 'high';
  changeRate: number; // changes per second
  lastActivity: Date;
  suspendProcessing: boolean;
}

interface ActivityThresholds {
  low: number;
  medium: number;
  high: number;
  suspendThreshold: number;
}

export class ActivityDetector {
  private changeHistory: Map<string, number[]> = new Map();
  private activityWindow: number = 30000; // 30 seconds
  private thresholds: ActivityThresholds = {
    low: 2,    // < 2 changes/30s
    medium: 10, // 2-10 changes/30s  
    high: 20,   // > 10 changes/30s
    suspendThreshold: 50 // > 50 changes/30s = suspend
  };
  private lastActivityCheck: Date = new Date();

  recordChange(filePath: string): void {
    const now = Date.now();
    
    if (!this.changeHistory.has(filePath)) {
      this.changeHistory.set(filePath, []);
    }
    
    const fileHistory = this.changeHistory.get(filePath)!;
    fileHistory.push(now);
    
    // Clean old entries outside the window
    const cutoff = now - this.activityWindow;
    const recentChanges = fileHistory.filter(timestamp => timestamp > cutoff);
    this.changeHistory.set(filePath, recentChanges);
    
    this.lastActivityCheck = new Date();
  }

  getActivityState(): ActivityState {
    const now = Date.now();
    const cutoff = now - this.activityWindow;
    
    // Count total changes across all files in the window
    let totalChanges = 0;
    for (const [filePath, timestamps] of this.changeHistory.entries()) {
      const recentChanges = timestamps.filter(timestamp => timestamp > cutoff);
      totalChanges += recentChanges.length;
      
      // Update the stored history
      this.changeHistory.set(filePath, recentChanges);
    }
    
    const changeRate = totalChanges / (this.activityWindow / 1000); // changes per second
    
    let intensity: ActivityState['intensity'];
    let suspendProcessing = false;
    
    if (totalChanges >= this.thresholds.suspendThreshold) {
      intensity = 'high';
      suspendProcessing = true;
    } else if (totalChanges >= this.thresholds.high) {
      intensity = 'high';
    } else if (totalChanges >= this.thresholds.medium) {
      intensity = 'medium';
    } else {
      intensity = 'low';
    }
    
    return {
      isActive: totalChanges > 0,
      intensity,
      changeRate,
      lastActivity: this.lastActivityCheck,
      suspendProcessing
    };
  }

  shouldSuspendProcessing(): boolean {
    return this.getActivityState().suspendProcessing;
  }

  clearHistory(): void {
    this.changeHistory.clear();
  }

  getStats(): {
    totalFiles: number;
    totalChanges: number;
    averageChangesPerFile: number;
  } {
    const now = Date.now();
    const cutoff = now - this.activityWindow;
    
    let totalChanges = 0;
    let activeFiles = 0;
    
    for (const timestamps of this.changeHistory.values()) {
      const recentChanges = timestamps.filter(timestamp => timestamp > cutoff);
      if (recentChanges.length > 0) {
        totalChanges += recentChanges.length;
        activeFiles++;
      }
    }
    
    return {
      totalFiles: activeFiles,
      totalChanges,
      averageChangesPerFile: activeFiles > 0 ? totalChanges / activeFiles : 0
    };
  }
}

export { ActivityState, ActivityThresholds };