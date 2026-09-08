// Finite offline lifecycle adapted from probe-macos-app-sandbox.py.
const fs = require('node:fs');
const {spawnSync} = require('node:child_process');
const settings = JSON.parse(fs.readFileSync('build-settings.json', 'utf8'));
if (fs.readFileSync('prebuild', 'utf8') !== 'ok') throw Error('prebuild did not run');
function run(command, args, expected) {
  const result = spawnSync(command, args, {encoding: 'utf8', timeout: 10000, maxBuffer: 65536});
  if (result.error || !expected.includes(result.status)) {
    throw Error(JSON.stringify({command, status: result.status, error: result.error?.code, stderr: result.stderr}));
  }
  return result.status;
}
let command, prefix;
if (settings.kind === 'javascript') {
  fs.writeFileSync('generated.cjs', `
const fs = require('node:fs');
try { fs.writeFileSync(process.argv[2], process.argv[3]); }
catch (error) {
  if (!['EPERM', 'EACCES', 'EROFS'].includes(error.code)) throw error;
  process.exit(10);
}
`);
  command = process.execPath;
  prefix = ['generated.cjs'];
} else {
  fs.copyFileSync('native_writer.c', 'generated.c');
  run(settings.compiler, [...settings.compiler_args, 'generated.c', '-o', 'generated'], [0]);
  command = './generated';
  prefix = [];
}
run(command, [...prefix, 'artifact', 'built'], [0]);
const child = run(command, [...prefix, settings.target, 'changed'], [0, 10]);
let parent = 0;
try { fs.writeFileSync(settings.parent_target, 'changed'); }
catch (error) {
  if (!['EPERM', 'EACCES', 'EROFS'].includes(error.code)) throw error;
  parent = 10;
}
fs.writeFileSync('build-result.json', JSON.stringify({child, parent}));
