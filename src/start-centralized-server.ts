#!/usr/bin/env ts-node

import { CortexEmbeddingServer } from './cortex-embedding-server';
import { log, error } from './logging-utils';

/**
 * Startup script for Cortex V3.0 Centralized Embedding Server
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

async function startCentralizedServer(): Promise<void> {
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

    // Create and start server
    const server = new CortexEmbeddingServer(port);
    await server.start();

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
    error(`[StartupScript] Failed to start centralized server: ${err.message}`);
    console.error(`
❌ Failed to Start Cortex V3.0 Centralized Server

Error: ${err.message}

Common Issues:
  • Port ${process.env.CORTEX_EMBEDDING_SERVER_PORT || '8766'} already in use
  • Insufficient memory (requires ~2GB minimum)
  • Missing dependencies (run: npm install)
  • ProcessPool initialization failure

Solutions:
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