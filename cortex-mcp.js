#!/usr/bin/env node

/**
 * Cortex MCP Server - stdio transport
 * Entry point for Claude Code MCP integration
 * 
 * Usage: claude mcp add cortex npx cortex-mcp
 */

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

// Helper to get repository path
function getRepoPath() {
  try {
    // Use current working directory where claude was launched
    const cwd = process.cwd();
    
    // Verify it's a git repository or has project files
    const hasGit = fs.existsSync(path.join(cwd, '.git'));
    const hasPackageJson = fs.existsSync(path.join(cwd, 'package.json'));
    const hasClaude = fs.existsSync(path.join(cwd, 'CLAUDE.md'));
    
    if (hasGit || hasPackageJson || hasClaude) {
      return cwd;
    }
    
    // Fallback to current directory
    return cwd;
  } catch (error) {
    return process.cwd();
  }
}

// Helper to get version info
function getVersion() {
  try {
    const packagePath = path.join(__dirname, 'package.json');
    const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'));
    return packageJson.version || 'unknown';
  } catch {
    return 'unknown';
  }
}

// Main execution
async function main() {
  const repoPath = getRepoPath();
  const version = getVersion();
  
  console.error(`[Cortex] Starting MCP Server v${version} for ${repoPath}`);
  console.error(`[Cortex] Mode: stdio transport`);
  
  try {
    // Determine the server script to use
    const stdioServerPath = path.join(__dirname, 'src', 'stdio-server.ts');
    const tsNodePath = path.join(__dirname, 'node_modules', '.bin', 'ts-node');
    
    // Check if we're in development (source available) or production (compiled)
    const isDev = fs.existsSync(path.join(__dirname, 'src'));
    
    if (isDev && fs.existsSync(tsNodePath)) {
      // Development mode - use ts-node
      const child = spawn(tsNodePath, [stdioServerPath, repoPath, ...process.argv.slice(2)], {
        stdio: 'inherit',
        env: { 
          ...process.env,
          CORTEX_REPO_PATH: repoPath,
          CORTEX_ENABLE_NEW_LOGGING: 'true'
        }
      });
      
      child.on('error', (error) => {
        console.error(`[Cortex] Error starting server: ${error.message}`);
        process.exit(1);
      });
      
      child.on('exit', (code) => {
        process.exit(code || 0);
      });
      
    } else {
      // Production mode - use compiled JS
      const distPath = path.join(__dirname, 'dist', 'stdio-server.js');
      
      if (fs.existsSync(distPath)) {
        const child = spawn('node', [distPath, repoPath, ...process.argv.slice(2)], {
          stdio: 'inherit',
          env: { 
            ...process.env,
            CORTEX_REPO_PATH: repoPath
          }
        });
        
        child.on('error', (error) => {
          console.error(`[Cortex] Error starting server: ${error.message}`);
          process.exit(1);
        });
        
        child.on('exit', (code) => {
          process.exit(code || 0);
        });
      } else {
        console.error('[Cortex] Error: Compiled server not found. Run npm run build first.');
        process.exit(1);
      }
    }
  } catch (error) {
    console.error(`[Cortex] Error starting server: ${error.message}`);
    process.exit(1);
  }
}

// Handle process signals
process.on('SIGINT', () => {
  console.error('[Cortex] Received SIGINT, shutting down...');
  process.exit(0);
});

process.on('SIGTERM', () => {
  console.error('[Cortex] Received SIGTERM, shutting down...');
  process.exit(0);
});

// Start the server
if (require.main === module) {
  main().catch((error) => {
    console.error(`[Cortex] Fatal error: ${error.message}`);
    process.exit(1);
  });
}