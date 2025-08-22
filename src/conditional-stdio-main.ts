/**
 * Conditional stdio MCP Server - Main Entry Point
 * Only activates when Claude Code is running
 */

import { ConditionalStdioServer } from './conditional-stdio-server';
import { conditionalLogger } from './utils/console-logger';
import { cortexConfig } from './env-config';
import * as os from 'os';
import { execSync } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';

// Helper functions
function getGitCommit(): string {
  try {
    return execSync('git rev-parse --short HEAD', { encoding: 'utf8' }).trim();
  } catch {
    return 'unknown';
  }
}

function getVersion(): string {
  try {
    const packagePath = path.join(__dirname, '../package.json');
    const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
    return packageJson.version || 'unknown';
  } catch {
    return 'unknown';
  }
}

// Parse command line arguments
const args = process.argv.slice(2);
const repoPath = args.find(arg => !arg.startsWith('--')) || process.env.CORTEX_REPO_PATH || process.cwd();
const forceReindex = args.includes('--reindex') || args.includes('--force-rebuild') || cortexConfig.forceRebuild;
const forceFullMode = args.includes('--full');
const enableRealTime = !args.includes('--no-watch') && !cortexConfig.disableRealTime;

// Determine indexing mode
let indexMode: 'full' | 'incremental' | 'reindex';
if (forceReindex || cortexConfig.indexMode === 'reindex') {
  indexMode = 'reindex';
} else if (forceFullMode) {
  indexMode = 'full';
} else if (cortexConfig.indexMode) {
  indexMode = cortexConfig.indexMode as 'full' | 'incremental';
} else {
  indexMode = 'incremental'; // Default for conditional server
}

async function main() {
  // Startup metadata
  const version = getVersion();
  const commit = getGitCommit();
  const nodeVersion = process.version;
  const platform = os.platform();
  const pid = process.pid;

  conditionalLogger.ready(`Cortex MCP Server (Conditional) v${version} (${commit})`, {
    metadata: {
      pid,
      node: nodeVersion,
      platform,
      repo: repoPath,
      mode: 'conditional-activation'
    }
  });

  try {
    // Create conditional server
    const conditionalServer = new ConditionalStdioServer(repoPath, {
      indexMode,
      enableRealTime
    });

    // Start conditional monitoring
    await conditionalServer.startConditionalMonitoring();

    conditionalLogger.ok('🎯 Cortex MCP Server ready - waiting for Claude Code...', {
      metadata: {
        repoPath,
        indexMode,
        realTime: enableRealTime
      }
    });

    // Handle graceful shutdown
    process.on('SIGINT', async () => {
      conditionalLogger.start('Received SIGINT, initiating graceful shutdown');
      await conditionalServer.stopConditionalMonitoring();
      process.exit(0);
    });

    process.on('SIGTERM', async () => {
      conditionalLogger.start('Received SIGTERM, initiating graceful shutdown');
      await conditionalServer.stopConditionalMonitoring();
      process.exit(0);
    });

    // Keep the process alive
    const keepAlive = setInterval(() => {
      // Log status every 5 minutes
      const status = conditionalServer.getStatus();
      if (status.isServerActive) {
        conditionalLogger.start('🟢 MCP Server active', {
          metadata: {
            claudeProcesses: status.claudeStatus.processCount
          }
        });
      } else {
        conditionalLogger.start('🔄 Waiting for Claude Code...', {
          metadata: {
            monitoring: status.claudeStatus.isMonitoring
          }
        });
      }
    }, 5 * 60 * 1000); // 5 minutes

    // Cleanup interval on exit
    process.on('exit', () => {
      clearInterval(keepAlive);
    });

  } catch (error: any) {
    conditionalLogger.fail('Failed to start Cortex Conditional MCP Server', {
      metadata: { error: error.message }
    });
    process.exit(1);
  }
}

// Start the conditional server
if (require.main === module) {
  main().catch((error: any) => {
    console.error('Fatal error starting conditional stdio server:', error);
    process.exit(1);
  });
}