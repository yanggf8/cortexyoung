export interface ParsedArgs {
  command: string;
  projectRoot: string;
  flags: {
    json: boolean;
    help: boolean;
    version: boolean;
    force: boolean;
  };
  options: {
    url: string;
  };
}

export function parseArgs(argv: string[]): ParsedArgs {
  const args = argv.slice(2);
  let command = '';
  let projectRoot = process.cwd();
  const flags = { json: false, help: false, version: false, force: false };
  const options = { url: '' };

  for (let i = 0; i < args.length; i++) {
    switch (args[i]) {
      case '--root':
        projectRoot = args[++i] || process.cwd();
        break;
      case '--json':
        flags.json = true;
        break;
      case '--help':
      case '-h':
        flags.help = true;
        break;
      case '--version':
      case '-v':
        flags.version = true;
        break;
      case '--force':
        flags.force = true;
        break;
      case '--url':
        options.url = args[++i] || '';
        break;
      default:
        if (!command && !args[i].startsWith('-')) {
          command = args[i];
        }
        break;
    }
  }

  return { command, projectRoot, flags, options };
}
