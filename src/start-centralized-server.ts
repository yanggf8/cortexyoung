#!/usr/bin/env ts-node

import { CortexEmbeddingServer } from './cortex-embedding-server';
import { log, error } from './logging-utils';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as net from 'net';

/**
 * Startup script for Cortex V3.0 Centralized Embedding Server
 * 
 * Features:
 *   - Singleton enforcement (prevents multiple servers)
 *   - Proper child process cleanup on shutdown
 *   - PID file management
 *   - Port conflict detection
 * 
 * Usage:
 *   npm run start:centralized              # Default port 8766
 *   npm run start:centralized -- 8777     # Custom port
 *   
 * Environment Variables:
 *   CORTEX_EMBEDDING_SERVER_PORT - Port to run on (default: 8766)
 *   CORTEX_ENABLE_NEW_LOGGING - Enable enhanced logging
 *   NODE_ENV - Environment (development/production)
 */

const CORTEX_PID_DIR = path.join(os.homedir(), '.cortex');
const CORTEX_PID_FILE = path.join(CORTEX_PID_DIR, 'centralized-server.pid');

interface ServerInstance {
  pid: number;
  port: number;
  startTime: number;
}

/**
 * Check if port is available
 */
function isPortAvailable(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = net.createServer();
    
    server.listen(port, () => {
      server.once('close', () => {
        resolve(true);
      });
      server.close();
    });
    
    server.on('error', () => {
      resolve(false);
    });
  });
}

/**
 * Check if process is still running
 */
function isProcessRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return false;
  }
}

/**
 * Check for existing centralized server instance
 */
function checkExistingInstance(port: number): ServerInstance | null {
  try {
    if (!fs.existsSync(CORTEX_PID_FILE)) {
      return null;
    }

    const pidData = fs.readFileSync(CORTEX_PID_FILE, 'utf8');
    const instance: ServerInstance = JSON.parse(pidData);
    
    // Check if the process is still running
    if (isProcessRunning(instance.pid)) {
      // Verify it's actually listening on the expected port
      if (instance.port === port) {
        return instance;
      }
    }
    
    // Stale PID file - remove it
    fs.unlinkSync(CORTEX_PID_FILE);
    return null;
    
  } catch (err) {
    // Corrupt PID file - remove it
    try {
      fs.unlinkSync(CORTEX_PID_FILE);
    } catch {}
    return null;
  }
}

/**
 * Create PID file for current instance
 */
function createPidFile(port: number): void {
  try {
    if (!fs.existsSync(CORTEX_PID_DIR)) {
      fs.mkdirSync(CORTEX_PID_DIR, { recursive: true });
    }

    const instance: ServerInstance = {
      pid: process.pid,
      port: port,
      startTime: Date.now()
    };

    fs.writeFileSync(CORTEX_PID_FILE, JSON.stringify(instance, null, 2));
    log(`[StartupScript] Created PID file: ${CORTEX_PID_FILE}`);
  } catch (err) {
    error(`[StartupScript] Failed to create PID file: ${err instanceof Error ? err.message : String(err)}`);
  }
}

/**
 * Remove PID file on shutdown
 */
function removePidFile(): void {
  try {
    if (fs.existsSync(CORTEX_PID_FILE)) {
      fs.unlinkSync(CORTEX_PID_FILE);
      log(`[StartupScript] Removed PID file: ${CORTEX_PID_FILE}`);
    }
  } catch (err) {
    error(`[StartupScript] Failed to remove PID file: ${err instanceof Error ? err.message : String(err)}`);
  }
}

async function startCentralizedServer(): Promise<void> {
  let server: CortexEmbeddingServer | null = null;
  
  // Setup cleanup handlers FIRST
  const cleanup = async () => {
    log(`[StartupScript] Shutting down centralized server...`);
    
    if (server) {
      try {
        await server.shutdown();
        log(`[StartupScript] Server shutdown complete`);
      } catch (err) {
        error(`[StartupScript] Error during server shutdown: ${err instanceof Error ? err.message : String(err)}`);
      }
    }
    
    removePidFile();
    log(`[StartupScript] Cleanup complete - exiting`);
  };

  // Register cleanup handlers for graceful shutdown
  process.on('SIGINT', async () => {
    log(`[StartupScript] Received SIGINT - graceful shutdown`);
    await cleanup();
    process.exit(0);
  });

  process.on('SIGTERM', async () => {
    log(`[StartupScript] Received SIGTERM - graceful shutdown`);
    await cleanup();
    process.exit(0);
  });

  process.on('SIGHUP', async () => {
    log(`[StartupScript] Received SIGHUP - graceful shutdown`);
    await cleanup();
    process.exit(0);
  });

  try {
    // Parse command line arguments
    const args = process.argv.slice(2);
    const customPort = args[0] ? parseInt(args[0], 10) : null;
    
    // Determine port
    const port = customPort || 
                parseInt(process.env.CORTEX_EMBEDDING_SERVER_PORT || '8766', 10);

    if (isNaN(port) || port < 1 || port > 65535) {
      throw new Error(`Invalid port: ${port}. Must be between 1 and 65535`);
    }

    // Check for existing instance (singleton enforcement)
    const existingInstance = checkExistingInstance(port);
    if (existingInstance) {
      const uptimeMs = Date.now() - existingInstance.startTime;
      const uptimeMinutes = Math.round(uptimeMs / 60000);
      
      console.error(`
❌ Centralized Server Already Running!

Another Cortex V3.0 Centralized Server is already running:
  • PID: ${existingInstance.pid}
  • Port: ${existingInstance.port}
  • Uptime: ${uptimeMinutes} minutes
  • Started: ${new Date(existingInstance.startTime).toLocaleString()}

Solutions:
  1. Use the existing server (recommended)
  2. Stop existing server: kill ${existingInstance.pid}
  3. Use different port: npm run start:centralized -- ${port + 1}

Only ONE centralized server should run per system for resource consolidation.
`);
      process.exit(1);
    }

    // Check if port is available
    const portAvailable = await isPortAvailable(port);
    if (!portAvailable) {
      throw new Error(`Port ${port} is already in use by another process. Try a different port with: npm run start:centralized -- ${port + 1}`);
    }

    // Display startup banner
    console.log(`
╭─────────────────────────────────────────────────────╮
│                                                     │
│    🚀 Cortex V3.0 Centralized Embedding Server     │
│                                                     │
│    Revolutionary Architecture:                      │
│    • N Claude instances × 8 processes              │
│      → Single HTTP server with 8 shared processes  │
│                                                     │
│    • Context Enhancement Layer                      │
│    • Memory-Mapped Cache                            │
│    • Adaptive Resource Management                   │
│    • Real-time Monitoring Dashboard                 │
│                                                     │
╰─────────────────────────────────────────────────────╯
`);

    log(`[StartupScript] Starting Cortex V3.0 Centralized Server on port ${port}`);
    log(`[StartupScript] Environment: ${process.env.NODE_ENV || 'development'}`);
    log(`[StartupScript] Enhanced Logging: ${process.env.CORTEX_ENABLE_NEW_LOGGING || 'false'}`);

    // Create PID file BEFORE starting server
    createPidFile(port);

    // Create and start server
    server = new CortexEmbeddingServer(port);
    await server.start();

    log(`[StartupScript] Server successfully started and registered with PID ${process.pid}`);

    // Display success information
    console.log(`
✅ Cortex V3.0 Centralized Server Successfully Started!

📊 Monitoring Endpoints:
   • Dashboard:  http://localhost:${port}/dashboard
   • Status:     http://localhost:${port}/status  
   • Health:     http://localhost:${port}/health
   • Metrics:    http://localhost:${port}/metrics

🔗 API Endpoints:
   • Embedding:           POST http://localhost:${port}/embed
   • Semantic Search:     POST http://localhost:${port}/semantic-search-enhanced  
   • Code Intelligence:   POST http://localhost:${port}/code-intelligence
   • Relationship Anal:   POST http://localhost:${port}/relationship-analysis
   • Execution Trace:     POST http://localhost:${port}/trace-execution-path
   • Code Patterns:       POST http://localhost:${port}/find-code-patterns

💡 Usage Examples:
   curl -X GET http://localhost:${port}/health
   curl -X GET http://localhost:${port}/dashboard

🛑 Shutdown:
   Press Ctrl+C for graceful shutdown
`);

    // Display memory and CPU information
    const totalMemoryGB = Math.round(process.memoryUsage().rss / (1024 * 1024 * 1024) * 100) / 100;
    const totalSystemMemoryGB = Math.round(require('os').totalmem() / (1024 * 1024 * 1024));
    const cpuCount = require('os').cpus().length;
    
    log(`[StartupScript] System Resources:`);
    log(`[StartupScript]   Memory: ${totalMemoryGB}GB used / ${totalSystemMemoryGB}GB total`);
    log(`[StartupScript]   CPU: ${cpuCount} cores available`);
    log(`[StartupScript]   Process: ${process.pid}`);

  } catch (err) {
    error(`[StartupScript] Failed to start centralized server: ${err instanceof Error ? err.message : String(err)}`);
    
    // Cleanup on startup failure
    removePidFile();
    
    if (server) {
      try {
        await server.shutdown();
      } catch (shutdownErr) {
        error(`[StartupScript] Error during cleanup shutdown: ${shutdownErr instanceof Error ? shutdownErr.message : String(shutdownErr)}`);
      }
    }
    
    console.error(`
❌ Failed to Start Cortex V3.0 Centralized Server

Error: ${err instanceof Error ? err.message : String(err)}

Common Issues:
  • Port ${process.env.CORTEX_EMBEDDING_SERVER_PORT || '8766'} already in use
  • Another centralized server already running (check with ps aux | grep centralized)
  • Insufficient memory (requires ~2GB minimum)
  • Missing dependencies (run: npm install)
  • ProcessPool initialization failure

Solutions:
  • Check for existing server: cat ~/.cortex/centralized-server.pid
  • Try a different port: npm run start:centralized -- 8777
  • Check available memory: free -h
  • Verify dependencies: npm audit
  • Check logs above for specific errors
`);
    
    process.exit(1);
  }
}

// Handle uncaught exceptions gracefully
process.on('unhandledRejection', (reason, promise) => {
  error(`[StartupScript] Unhandled promise rejection:`, reason);
  process.exit(1);
});

process.on('uncaughtException', (err) => {
  error(`[StartupScript] Uncaught exception:`, err);
  process.exit(1);
});

// Start the server
startCentralizedServer();