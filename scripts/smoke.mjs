import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const executable = fileURLToPath(new URL('../target/debug/rig', import.meta.url));
const webDirectory = fileURLToPath(new URL('../web/dist', import.meta.url));
const child = spawn(executable, ['serve', '--port', '0', '--web-dir', webDirectory], {
  cwd: repoRoot,
  stdio: ['ignore', 'pipe', 'pipe'],
});

let stderr = '';
child.stderr.on('data', (chunk) => { stderr += chunk; });
const lines = createInterface({ input: child.stdout });
const exit = once(child, 'exit');

async function within(promise, milliseconds, message) {
  let timeout;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(() => reject(new Error(`${message}\n${stderr}`)), milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timeout);
  }
}

try {
  const [announcement] = await within(Promise.race([
    once(lines, 'line'),
    exit.then(([code, signal]) => {
      assert.fail(`Server exited before startup: code=${code}, signal=${signal}.\n${stderr}`);
    }),
  ]), 10_000, 'Server did not announce its address within 10 seconds.');
  assert.match(announcement, /^Rig is listening at http:\/\/127\.0\.0\.1:\d+$/);
  const baseUrl = announcement.slice('Rig is listening at '.length);

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
  try {
    assert.ok(child.kill('SIGINT'), 'Server must remain running until explicitly stopped.');
    const [code, signal] = await within(exit, 5_000, 'Server did not stop within 5 seconds.');
    assert.equal(code, 0, `Server exited unsuccessfully.\n${stderr}`);
    assert.equal(signal, null, 'Server must handle SIGINT with a normal exit.');
  } finally {
    lines.close();
    child.stdout.destroy();
    child.stderr.destroy();
    child.unref();
  }
}
