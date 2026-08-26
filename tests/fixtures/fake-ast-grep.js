#!/usr/bin/env node
// Scriptable stand-in for ast-grep, driven by FAKE_AG_MODE.
// Only used for pathological paths; happy paths use the real binary.
const mode = process.env.FAKE_AG_MODE ?? '';
const args = process.argv.slice(2);

if (args[0] === '--version') {
  const v = mode.startsWith('version:') ? mode.slice('version:'.length) : '0.45.2';
  process.stdout.write(`ast-grep ${v}\n`);
  process.exit(0);
}
if (mode === 'hang') { setTimeout(() => {}, 60_000); }
else if (mode === 'streams') {
  process.stdout.write('OUT\n'); process.stderr.write('ERR\n'); process.exit(1);
} else if (mode === 'empty') { process.exit(1); }
else if (mode.startsWith('emit:')) {
  process.stdout.write(Buffer.from(mode.slice('emit:'.length), 'base64').toString('utf8'));
  process.exit(0);
} else if (mode.startsWith('preflight-bad')) {
  process.stderr.write('Debug AST:\nprogram (0,0)-(0,10)\n  ERROR (0,0)-(0,10)\n\n' +
    'Warning: Pattern contains an ERROR node and may cause unexpected results.\n');
  process.exit(0);
} else if (mode === 'preflight-ok') {
  process.stderr.write('Debug AST:\nprogram (0,0)-(0,9)\n'); process.exit(0);
} else { process.exit(0); }
