#!/usr/bin/env node

import { parseArgs } from './cli/args.js';
import { runScan, runUpdate, runStatus, runLogin, runSync } from './cli/commands.js';
import { readFileSync } from 'fs';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const USAGE = `
Usage: claudecat <command> [options]

Commands:
  scan      Analyze project and display detected patterns
  update    Analyze project and update CLAUDE.md
  status    Show current detected patterns and confidence scores
  login     Configure Turso cloud storage credentials
  sync      Push/pull patterns to/from cloud storage

Options:
  --root <path>   Project root directory (default: cwd)
  --json          Output as JSON
  --force         Force sync regardless of timestamps
  --url <url>     Turso database URL (for login)
  --help, -h      Show this help message
  --version, -v   Show version

Environment:
  TURSO_AUTH_TOKEN  Turso auth token (avoids interactive prompt)

Examples:
  claudecat scan                                    # Analyze current project
  claudecat update                                  # Update CLAUDE.md
  claudecat status --json                           # Machine-readable output
  claudecat login --url libsql://...                # Configure cloud (prompts for token)
  TURSO_AUTH_TOKEN=... claudecat login --url ...    # Non-interactive login
  claudecat sync                                    # Sync patterns to cloud
  claudecat sync --force                            # Force push local patterns
`;

async function main(): Promise<void> {
  const { command, projectRoot, flags, options } = parseArgs(process.argv);

  if (flags.version) {
    try {
      const __dirname = dirname(fileURLToPath(import.meta.url));
      const pkg = JSON.parse(readFileSync(join(__dirname, '..', 'package.json'), 'utf-8'));
      console.log(`claudecat v${pkg.version}`);
    } catch {
      console.log('claudecat v1.0.0');
    }
    return;
  }

  if (flags.help || !command) {
    console.log(USAGE.trim());
    return;
  }

  switch (command) {
    case 'scan':
      await runScan(projectRoot, flags);
      break;
    case 'update':
      await runUpdate(projectRoot, flags);
      break;
    case 'status':
      await runStatus(projectRoot, flags);
      break;
    case 'login':
      await runLogin(options);
      break;
    case 'sync':
      await runSync(projectRoot, flags);
      break;
    default:
      console.error(`Unknown command: ${command}\n`);
      console.log(USAGE.trim());
      process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(`Error: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
