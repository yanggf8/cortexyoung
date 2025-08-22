/**
 * Claude Code Process Detection
 * Monitors system for Claude Code process launch/exit events
 */

import { execSync } from 'child_process';
import { EventEmitter } from 'events';
import { conditionalLogger } from './utils/console-logger';

export interface ClaudeProcessInfo {
  pid: number;
  command: string;
  startTime: Date;
}

export class ClaudeProcessDetector extends EventEmitter {
  private monitorInterval: NodeJS.Timeout | null = null;
  private lastKnownProcesses: Set<number> = new Set();
  private isMonitoring = false;
  private readonly checkIntervalMs: number = 2000; // Check every 2 seconds

  constructor() {
    super();
  }

  /**
   * Start monitoring for Claude Code processes
   */
  startMonitoring(): void {
    if (this.isMonitoring) {
      conditionalLogger.warn('Claude process detector already monitoring');
      return;
    }

    this.isMonitoring = true;
    conditionalLogger.start('🔍 Starting Claude Code process monitoring', {
      metadata: { interval: `${this.checkIntervalMs}ms` }
    });

    // Initial scan
    this.scanForClaudeProcesses();

    // Set up periodic monitoring
    this.monitorInterval = setInterval(() => {
      this.scanForClaudeProcesses();
    }, this.checkIntervalMs);
  }

  /**
   * Stop monitoring for Claude Code processes
   */
  stopMonitoring(): void {
    if (!this.isMonitoring) {
      return;
    }

    this.isMonitoring = false;
    
    if (this.monitorInterval) {
      clearInterval(this.monitorInterval);
      this.monitorInterval = null;
    }

    conditionalLogger.ok('✅ Claude Code process monitoring stopped');
  }

  /**
   * Check if Claude Code is currently running
   */
  isClaudeCodeRunning(): boolean {
    const processes = this.getCurrentClaudeProcesses();
    return processes.length > 0;
  }

  /**
   * Get current Claude Code processes
   */
  getCurrentClaudeProcesses(): ClaudeProcessInfo[] {
    try {
      // Look for Claude Code processes - check multiple possible process names
      const commands = [
        'ps aux | grep -E "(claude|claude-code|claude_code)" | grep -v grep | grep -v stdio-server',
        'ps aux | grep -E "node.*claude" | grep -v grep | grep -v stdio-server',
        'pgrep -f "claude" 2>/dev/null || echo ""'
      ];

      const processes: ClaudeProcessInfo[] = [];

      for (const command of commands) {
        try {
          const result = execSync(command, { encoding: 'utf8', timeout: 5000 });
          const lines = result.trim().split('\n').filter(line => line.length > 0);

          for (const line of lines) {
            const match = line.match(/^\w+\s+(\d+)/);
            if (match) {
              const pid = parseInt(match[1]);
              if (pid && !isNaN(pid)) {
                processes.push({
                  pid,
                  command: line.trim(),
                  startTime: new Date()
                });
              }
            }
          }
        } catch (error) {
          // Ignore individual command failures
        }
      }

      // Remove duplicates by PID
      const uniqueProcesses = processes.filter((proc, index, arr) => 
        arr.findIndex(p => p.pid === proc.pid) === index
      );

      return uniqueProcesses;
    } catch (error) {
      conditionalLogger.warn('Error detecting Claude processes', {
        metadata: { error: error instanceof Error ? error.message : String(error) }
      });
      return [];
    }
  }

  /**
   * Scan for Claude Code processes and emit events on changes
   */
  private scanForClaudeProcesses(): void {
    try {
      const currentProcesses = this.getCurrentClaudeProcesses();
      const currentPids = new Set(currentProcesses.map(p => p.pid));

      // Check for new processes (launched)
      for (const process of currentProcesses) {
        if (!this.lastKnownProcesses.has(process.pid)) {
          conditionalLogger.ok('🚀 Claude Code launched', {
            metadata: { 
              pid: process.pid,
              command: process.command.substring(0, 80) + '...'
            }
          });
          this.emit('claude-launched', process);
        }
      }

      // Check for terminated processes (exited)
      for (const pid of this.lastKnownProcesses) {
        if (!currentPids.has(pid)) {
          conditionalLogger.start('🛑 Claude Code exited', {
            metadata: { pid }
          });
          this.emit('claude-exited', { pid });
        }
      }

      // Update known processes
      this.lastKnownProcesses = currentPids;

    } catch (error) {
      conditionalLogger.warn('Error scanning for Claude processes', {
        metadata: { error: error instanceof Error ? error.message : String(error) }
      });
    }
  }

  /**
   * Get monitoring status
   */
  getStatus(): { isMonitoring: boolean; processCount: number; processes: ClaudeProcessInfo[] } {
    const processes = this.getCurrentClaudeProcesses();
    return {
      isMonitoring: this.isMonitoring,
      processCount: processes.length,
      processes
    };
  }
}

// Singleton instance for global use
export const claudeProcessDetector = new ClaudeProcessDetector();