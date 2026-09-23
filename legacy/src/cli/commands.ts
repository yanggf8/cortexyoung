import { createInterface } from 'readline';
import { EnhancedProjectDetector } from '../core/project-detector.js';
import { CLAUDEMdMaintainer } from '../core/claude-md-maintainer.js';
import { TursoClient } from '../cloud/turso-client.js';
import { ConfigStore } from '../cloud/config-store.js';
import { ProjectIdentifier } from '../cloud/project-identifier.js';
import { getMachineId } from '../cloud/machine-id.js';
import { mergeSnapshots } from '../cloud/pattern-merger.js';
import { formatScanResult, formatStatus } from './formatter.js';
import type { ParsedArgs } from './args.js';

export async function runScan(projectRoot: string, flags: ParsedArgs['flags']): Promise<void> {
  const detector = new EnhancedProjectDetector(projectRoot);
  const context = detector.detectCurrentContext();

  if (flags.json) {
    process.stdout.write(JSON.stringify(context, null, 2) + '\n');
  } else {
    console.log(formatScanResult(context));
  }
}

export async function runUpdate(projectRoot: string, flags: ParsedArgs['flags']): Promise<void> {
  const detector = new EnhancedProjectDetector(projectRoot);
  const context = detector.detectCurrentContext();
  const maintainer = new CLAUDEMdMaintainer(projectRoot);

  await maintainer.updateProjectContext(context);

  if (flags.json) {
    process.stdout.write(JSON.stringify({ updated: true, context }, null, 2) + '\n');
  }
}

export async function runStatus(projectRoot: string, flags: ParsedArgs['flags']): Promise<void> {
  const detector = new EnhancedProjectDetector(projectRoot);
  const context = detector.detectCurrentContext();

  if (flags.json) {
    process.stdout.write(JSON.stringify(context, null, 2) + '\n');
  } else {
    console.log(formatStatus(context));
  }
}

function prompt(question: string): Promise<string> {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  return new Promise((resolve) => {
    rl.question(question, (answer) => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

function promptSecret(question: string): Promise<string> {
  return new Promise((resolve) => {
    process.stdout.write(question);
    const stdin = process.stdin;
    const wasRaw = stdin.isRaw;
    if (stdin.isTTY) stdin.setRawMode(true);
    let input = '';
    const onData = (chunk: Buffer) => {
      const char = chunk.toString();
      if (char === '\n' || char === '\r') {
        stdin.removeListener('data', onData);
        if (stdin.isTTY && wasRaw !== undefined) stdin.setRawMode(wasRaw);
        process.stdout.write('\n');
        resolve(input.trim());
      } else if (char === '\x03') {
        // Ctrl+C
        process.exit(1);
      } else if (char === '\x7f' || char === '\b') {
        input = input.slice(0, -1);
      } else {
        input += char;
      }
    };
    stdin.resume();
    stdin.on('data', onData);
  });
}

export async function runLogin(options: ParsedArgs['options']): Promise<void> {
  // Token: env var > interactive prompt (never CLI flag)
  let url = options.url;
  let token = process.env['TURSO_AUTH_TOKEN'] || '';

  if (!url) url = await prompt('Turso database URL: ');
  if (!token) token = await promptSecret('Turso auth token: ');

  if (!url || !token) {
    console.error('URL and token are required. Set TURSO_AUTH_TOKEN env var or enter interactively.');
    process.exitCode = 1;
    return;
  }

  // Validate connection
  const client = new TursoClient(url, token);
  try {
    await client.migrate();
    console.log('Connection verified. Schema ready.');
  } catch (error) {
    console.error(`Authentication failed: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
    return;
  } finally {
    client.close();
  }

  ConfigStore.write({ tursoUrl: url, tursoToken: token });
  console.log('Credentials saved to ~/.claudecat/config.json');
}

export async function runSync(projectRoot: string, flags: ParsedArgs['flags']): Promise<void> {
  const config = ConfigStore.read();
  if (!config) {
    console.error('Not configured. Run: claudecat login --url <turso-url>');
    process.exitCode = 1;
    return;
  }

  const client = new TursoClient(config.tursoUrl, config.tursoToken);
  try {
    await client.migrate();

    const { id, displayName } = ProjectIdentifier.resolve(projectRoot);
    const machineId = getMachineId();

    // Scan and push local patterns
    const detector = new EnhancedProjectDetector(projectRoot);
    const localContext = detector.detectCurrentContext();

    await client.upsertProject(id, displayName);
    await client.pushSnapshot(id, machineId, localContext);

    const maintainer = new CLAUDEMdMaintainer(projectRoot);

    if (flags.force) {
      // --force: skip merge, use local patterns only
      await maintainer.updateProjectContext(localContext);
      ConfigStore.setLastSyncedAt(id, localContext.lastUpdated);

      if (flags.json) {
        process.stdout.write(
          JSON.stringify({ action: 'force-pushed', project: id, context: localContext }, null, 2) + '\n',
        );
      } else {
        console.log(`Force-synced: ${displayName} — local patterns pushed (no merge)`);
      }
    } else {
      // Default: fetch all snapshots and merge
      const allSnapshots = await client.getAllSnapshots(id);
      const merged = mergeSnapshots(allSnapshots);

      await maintainer.updateProjectContext(merged);
      ConfigStore.setLastSyncedAt(id, merged.lastUpdated);

      if (flags.json) {
        process.stdout.write(
          JSON.stringify({ action: 'merged', machines: allSnapshots.length, project: id, context: merged }, null, 2) + '\n',
        );
      } else {
        console.log(`Synced: ${displayName} — merged patterns from ${allSnapshots.length} machine(s)`);
      }
    }
  } catch (error) {
    console.error(`Sync failed: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  } finally {
    client.close();
  }
}
