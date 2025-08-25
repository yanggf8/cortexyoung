#!/usr/bin/env node

/**
 * Cortex MCP Global Entry Point
 * Accessible from any project directory
 * 
 * Usage: claude mcp add cortex "node /path/to/cortex-mcp-global.js"
 */

const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

// Fixed path to the cortexyoung directory
const CORTEX_DIR = '/home/yanggf/a/cortexyoung';

// Get the working directory where Claude Code is running
const workingDir = process.cwd();

console.error(`[Cortex Global] Working directory: ${workingDir}`);
console.error(`[Cortex Global] Cortex directory: ${CORTEX_DIR}`);

// Launch the actual cortex MCP server from the fixed directory
const serverPath = path.join(CORTEX_DIR, 'cortex-multi-instance.js');

console.error(`[Cortex Global] Starting server: ${serverPath}`);

// Change to the cortex directory but pass the working directory as env var
process.env.CORTEX_WORKING_DIR = workingDir;
process.chdir(CORTEX_DIR);

// Execute the multi-instance server
require(serverPath);