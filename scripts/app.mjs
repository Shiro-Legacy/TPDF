// `npm run smoke` and `npm run package` for this OS, with no shell differences between them:
//   node scripts/app.mjs smoke [app arguments]   the release binary's self-test (`--smoke`): TANDEM_APP_BIN if set,
//                                                else crates/target/release/tandem-pdf (scripts/session.mjs)
//   node scripts/app.mjs package [script args]   scripts/package-mac.sh, package-windows.ps1 or package-linux.sh
//   node scripts/app.mjs ui                      the interface to WASM (crates/tandem-ui/build.sh --release), through
//                                                Git's sh on Windows, where npm runs scripts in cmd
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { appBinary } from './session.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const [command, ...args] = process.argv.slice(2);

function run(file, list) {
  const result = spawnSync(file, list, { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}

if (command === 'smoke') {
  const app = appBinary();
  if (!existsSync(app)) {
    console.error(`No app at ${app}: build it with \`cd crates && cargo build --release -p tandem-desktop\` (after npm run build:ui), or set TANDEM_APP_BIN.`);
    process.exit(1);
  }
  run(app, ['--smoke', ...args]);
} else if (command === 'ui') {
  let sh = 'sh';
  if (process.platform === 'win32') {
    // Git for Windows' sh, not System32's bash (WSL).
    const git = `${process.env.ProgramFiles ?? 'C:\\Program Files'}\\Git\\bin\\sh.exe`;
    if (existsSync(git)) sh = git;
  }
  run(sh, ['crates/tandem-ui/build.sh', '--release', ...args]);
} else if (command === 'package') {
  if (process.platform === 'darwin') run('zsh', ['scripts/package-mac.sh', ...args]);
  else if (process.platform === 'win32') run('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'scripts\\package-windows.ps1', ...args]);
  else if (process.platform === 'linux') run('bash', ['scripts/package-linux.sh', ...args]);
  else {
    console.error(`No package script for ${process.platform}.`);
    process.exit(1);
  }
} else {
  console.error('usage: node scripts/app.mjs smoke|package|ui [arguments]');
  process.exit(2);
}
