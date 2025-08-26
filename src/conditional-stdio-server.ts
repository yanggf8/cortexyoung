/**
 * Conditional stdio MCP Server
 * Only activates when Claude Code is running
 */

import { claudeProcessDetector, ClaudeProcessInfo } from './claude-process-detector';
import { LightweightStdioCortexMCPServer } from './stdio-server';
import { CodebaseIndexer } from './indexer';
import { ProjectManager } from './project-manager';
import { conditionalLogger } from './utils/console-logger';
import { cortexConfig } from './env-config';
import * as path from 'path';
import * as fs from 'fs';
import { execSync } from 'child_process';

export class ConditionalStdioServer {
  private mcpServer: LightweightStdioCortexMCPServer | null = null;
  private indexer: CodebaseIndexer | null = null;
  private projectManager: ProjectManager | null = null;
  private isServerActive = false;
  private repoPath: string;
  private serverConfig: {
    indexMode: 'full' | 'incremental' | 'reindex';
    enableRealTime: boolean;
  };

  constructor(repoPath: string, config: {
    indexMode?: 'full' | 'incremental' | 'reindex';
    enableRealTime?: boolean;
  } = {}) {
    this.repoPath = repoPath;
    this.serverConfig = {
      indexMode: config.indexMode || 'incremental',
      enableRealTime: config.enableRealTime ?? true
    };
  }

  /**
   * Start conditional monitoring
   */
  async startConditionalMonitoring(): Promise<void> {
    conditionalLogger.ready('🎯 Starting Conditional MCP Server', {
      metadata: {
        repo: this.repoPath,
        mode: 'claude-detection'
      }
    });

    // Set up event handlers
    claudeProcessDetector.on('claude-launched', this.handleClaudeLaunched.bind(this));
    claudeProcessDetector.on('claude-exited', this.handleClaudeExited.bind(this));

    // Check if Claude is already running
    const isAlreadyRunning = claudeProcessDetector.isClaudeCodeRunning();
    if (isAlreadyRunning) {
      conditionalLogger.ok('Claude Code already detected, starting MCP server');
      await this.startMCPServer();
    } else {
      conditionalLogger.start('Waiting for Claude Code launch...');
    }

    // Start monitoring
    claudeProcessDetector.startMonitoring();
  }

  /**
   * Stop conditional monitoring and cleanup
   */
  async stopConditionalMonitoring(): Promise<void> {
    conditionalLogger.start('🛑 Stopping conditional monitoring');

    // Stop monitoring
    claudeProcessDetector.stopMonitoring();

    // Stop MCP server if running
    if (this.isServerActive) {
      await this.stopMCPServer();
    }

    conditionalLogger.ok('✅ Conditional monitoring stopped');
  }

  /**
   * Handle Claude Code launch event
   */
  private async handleClaudeLaunched(processInfo: ClaudeProcessInfo): Promise<void> {
    if (this.isServerActive) {
      conditionalLogger.warn('MCP server already active, ignoring duplicate launch');
      return;
    }

    conditionalLogger.ok('🚀 Claude Code detected, activating MCP server', {
      metadata: { pid: processInfo.pid }
    });

    try {
      await this.startMCPServer();
    } catch (error) {
      conditionalLogger.fail('Failed to start MCP server on Claude launch', {
        metadata: { error: error instanceof Error ? error.message : String(error) }
      });
    }
  }

  /**
   * Handle Claude Code exit event
   */
  private async handleClaudeExited(exitInfo: { pid: number }): Promise<void> {
    // Check if any Claude processes are still running
    const stillRunning = claudeProcessDetector.isClaudeCodeRunning();
    
    if (stillRunning) {
      conditionalLogger.start('Other Claude Code instances still running, keeping MCP server active');
      return;
    }

    if (!this.isServerActive) {
      conditionalLogger.warn('MCP server not active, ignoring exit event');
      return;
    }

    conditionalLogger.start('🛑 All Claude Code instances exited, deactivating MCP server', {
      metadata: { lastPid: exitInfo.pid }
    });

    try {
      await this.stopMCPServer();
    } catch (error) {
      conditionalLogger.fail('Failed to stop MCP server on Claude exit', {
        metadata: { error: error instanceof Error ? error.message : String(error) }
      });
    }
  }

  /**
   * Start the MCP server with indexing
   */
  private async startMCPServer(): Promise<void> {
    if (this.isServerActive) {
      return;
    }

    conditionalLogger.start('🔄 Initializing Cortex components...', {
      metadata: { repo: this.repoPath }
    });

    // Initialize indexer
    this.indexer = new CodebaseIndexer(this.repoPath);
    
    // Index repository
    conditionalLogger.start('📚 Indexing repository...', {
      metadata: { mode: this.serverConfig.indexMode }
    });

    const indexResponse = await this.indexer.indexRepository({
      repository_path: this.repoPath,
      mode: this.serverConfig.indexMode
    });

    conditionalLogger.ok('📊 Indexing complete', {
      metadata: {
        chunks: indexResponse.chunks_processed,
        time: `${indexResponse.time_taken_ms}ms`
      }
    });

    // Initialize components
    const searcher = (this.indexer as any).searcher;
    this.projectManager = new ProjectManager();
    await this.projectManager.initializeWithCurrentDirectory();

    // Enable real-time updates if configured
    if (this.serverConfig.enableRealTime) {
      try {
        await this.indexer.enableRealTimeUpdates();
        conditionalLogger.ok('✅ Real-time file watching enabled');
      } catch (error) {
        conditionalLogger.warn('⚠️  Failed to enable real-time watching', {
          metadata: { error: error instanceof Error ? error.message : String(error) }
        });
      }
    }

    // Create and start MCP server
    this.mcpServer = new LightweightStdioCortexMCPServer(this.repoPath, this.projectManager);
    await this.mcpServer.start();

    this.isServerActive = true;
    conditionalLogger.ok('🎉 MCP Server activated and ready for Claude Code');
  }

  /**
   * Stop the MCP server and cleanup resources
   */
  private async stopMCPServer(): Promise<void> {
    if (!this.isServerActive || !this.mcpServer) {
      return;
    }

    conditionalLogger.start('🔄 Deactivating MCP server...');

    try {
      // Disable real-time updates
      if (this.indexer && this.serverConfig.enableRealTime) {
        await this.indexer.disableRealTimeUpdates();
      }

      // Stop MCP server
      await this.mcpServer.stop();

      this.mcpServer = null;
      this.indexer = null;
      this.projectManager = null;
      this.isServerActive = false;

      conditionalLogger.ok('✅ MCP Server deactivated');
    } catch (error) {
      conditionalLogger.fail('Error during MCP server shutdown', {
        metadata: { error: error instanceof Error ? error.message : String(error) }
      });
    }
  }

  /**
   * Get current status
   */
  getStatus(): {
    isServerActive: boolean;
    claudeStatus: any;
    repoPath: string;
    config: any;
  } {
    return {
      isServerActive: this.isServerActive,
      claudeStatus: claudeProcessDetector.getStatus(),
      repoPath: this.repoPath,
      config: this.serverConfig
    };
  }
}