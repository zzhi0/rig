import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const executable = fileURLToPath(new URL('../target/debug/rig', import.meta.url));
const webDirectory = fileURLToPath(new URL('../web/dist', import.meta.url));
const baseUrl = 'http://127.0.0.1:17878';
const child = spawn(executable, ['serve', '--port', '17878', '--web-dir', webDirectory], {
  cwd: repoRoot,
  stdio: ['ignore', 'pipe', 'pipe'],
});

let output = '';
let exited = false;
let spawnError;
child.stdout.on('data', (chunk) => { output += chunk; });
child.stderr.on('data', (chunk) => { output += chunk; });
const exit = new Promise((resolve) => {
  child.once('error', (error) => {
    spawnError = error;
    exited = true;
    resolve();
  });
  child.once('exit', () => {
    exited = true;
    resolve();
  });
});

try {
  const deadline = Date.now() + 10_000;
  let ready = false;
  while (Date.now() < deadline) {
    if (spawnError) throw spawnError;
    assert.ok(!exited, `Server exited before readiness.\n${output}`);
    try {
      const response = await fetch(`${baseUrl}/api/info`, {
        signal: AbortSignal.timeout(500),
      });
      ready = response.ok;
    } catch {
      // Connection refusal during startup is expected test synchronization.
    }
    if (ready) break;
    await delay(100);
  }
  assert.ok(ready, `Server did not become ready within 10 seconds.\n${output}`);

  const info = await fetch(`${baseUrl}/api/info`, { signal: AbortSignal.timeout(5_000) });
  assert.equal(info.status, 200);
  assert.match(info.headers.get('content-type'), /application\/json/);
  assert.deepEqual(await info.json(), { name: 'Rig', version: '0.1.0' });

  const page = await fetch(baseUrl, { signal: AbortSignal.timeout(5_000) });
  assert.equal(page.status, 200);
  assert.match(page.headers.get('content-type'), /text\/html/);
  const html = await page.text();
  assert.match(html, /<div\s+id="root"/);
  const assets = [...html.matchAll(/(?:src|href)="([^"]+\.(?:js|css))"/g)]
    .map((match) => match[1]);
  assert.ok(assets.some((asset) => asset.endsWith('.js')), 'HTML must reference built JavaScript.');
  assert.ok(assets.some((asset) => asset.endsWith('.css')), 'HTML must reference built CSS.');
  for (const asset of assets) {
    const response = await fetch(new URL(asset, baseUrl), { signal: AbortSignal.timeout(5_000) });
    assert.equal(response.status, 200, `Failed to serve ${asset}.`);
    assert.match(response.headers.get('content-type'), asset.endsWith('.js') ? /javascript/ : /text\/css/);
    assert.ok((await response.text()).trim().length > 0, `Empty asset: ${asset}.`);
  }

  console.log('Local API and built Web assets passed integration checks.');
} finally {
  if (!exited) {
    child.kill('SIGINT');
    let timeout;
    try {
      await Promise.race([
        exit,
        new Promise((_, reject) => {
          timeout = setTimeout(() => {
            child.kill('SIGKILL');
            reject(new Error(`Server did not stop within 5 seconds.\n${output}`));
          }, 5_000);
        }),
      ]);
    } finally {
      clearTimeout(timeout);
    }
  }
}
