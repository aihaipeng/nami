import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

if (process.platform !== 'darwin') {
  console.error('Native translation UI regression requires macOS.');
  process.exit(1);
}
const root = fileURLToPath(new URL('../../', import.meta.url));
const target = path.join(root, 'src-tauri/target/ui-tests');
const visible = process.env.NAMI_UI_TEST_VISIBLE === '1';
const report = path.join(target, visible ? 'native-ui-visible-report.json' : 'native-ui-report.json');
mkdirSync(target, { recursive: true });
const build = spawnSync('npm', ['run', 'tauri', '--', 'build', '--features', 'ui-tests', '--config', path.join(root, 'tests/native/tauri.ui-tests.conf.json')], {
  cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: target },
});
if (build.error) throw build.error;
if (build.status !== 0) process.exit(build.status ?? 1);
const executable = path.join(target, 'release/bundle/macos/Nami UI Tests.app/Contents/MacOS/nami');
const runId = `${process.pid}-${Date.now()}`;
const run = spawnSync(executable, [], {
  cwd: root, encoding: 'utf8', timeout: 90_000,
  env: { ...process.env, NAMI_UI_TEST_REPORT: report, NAMI_UI_TEST_RUN_ID: runId },
});
if (run.stdout) process.stdout.write(run.stdout);
if (run.stderr) process.stderr.write(run.stderr);
if (run.error) {
  console.error(run.error.message);
  process.exit(1);
}
if (!existsSync(report)) {
  console.error('Native test process did not produce its report.');
  process.exit(1);
}
const result = JSON.parse(readFileSync(report, 'utf8'));
if (result.runId !== runId) {
  console.error('Native UI report belongs to an older run.');
  process.exit(1);
}
for (const item of result.cases) console.log(`${item.passed ? 'PASS' : 'FAIL'} ${item.name} (${item.samples.length} samples)${item.error ? `: ${item.error}` : ''}`);
for (const item of result.domCases ?? []) console.log(`${item.passed ? 'PASS' : 'FAIL'} ${item.name} (${item.samples.length} native/DOM samples)${item.error ? `: ${item.error}` : ''}`);
console.log(`Native UI report: ${report}`);
console.log(`Scope: ${result.scope}`);
if (result.visiblePacedPresentation?.enabled) console.log(`Visible paced presentation: ${JSON.stringify(result.visiblePacedPresentation)}`);
console.log('OS mouse live-resize and material/remote-content visual quality remain unverified.');
process.exit(run.status === 0 && result.passed ? 0 : 1);
