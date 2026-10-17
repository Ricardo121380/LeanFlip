// Exercise the installed Windows app through its real WebView2 -> Tauri bridge.
// The caller supplies an isolated home and a loopback debugging port only for CI.
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { isAbsolute, join, relative, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const [port, home] = process.argv.slice(2);
assert.equal(process.platform, 'win32');
assert.match(port, /^\d+$/);
assert.ok(isAbsolute(home));
const timeout = setTimeout(() => { console.error('Windows WebView smoke timed out'); process.exit(1); }, 60_000);
let socket;
try {
  let page;
  for (let attempt = 0; attempt < 40; attempt++) {
    const pages = await fetch(`http://127.0.0.1:${port}/json/list`, { signal: AbortSignal.timeout(2000) })
      .then(response => response.json()).catch(() => []);
    page = pages.find(value => value.type === 'page' && new URL(value.url).hostname === 'tauri.localhost');
    if (page) break;
    await delay(500);
  }
  assert.ok(page, 'Installed app WebView2 page is available');
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await once(socket, 'open');
  let id = 0;
  const pending = new Map();
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  });
  const cdp = (method, params = {}) => new Promise((resolve, reject) => {
    pending.set(++id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async expression => {
    const result = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const invoke = (command, args = {}) => evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)}, ${JSON.stringify(args)})`);
  assert.match(await evaluate('document.body.innerText'), /LeanFlip/);
  const settings = await invoke('get_settings');
  assert.equal(resolve(settings.home).toLowerCase(), resolve(home).toLowerCase());
  assert.equal(resolve(settings.dataDir).toLowerCase(), resolve(home, '.leanflip').toLowerCase());
  const overview = await invoke('get_overview');
  assert.equal(overview.length, 10);
  for (const tool of overview) for (const path of tool.paths) {
    const child = relative(home, path);
    assert.ok(!isAbsolute(child) && !child.startsWith('..'), `Isolated tool path: ${tool.tool}`);
  }
  // Both switches use the actual frontend IPC channel and real local files.
  for (const id of ['windows-ci-a', 'windows-ci-b']) {
    await invoke('add_provider', {
      tool: 'claude', addToLive: false,
      provider: { id, name: id, settingsConfig: { env: {
        ANTHROPIC_BASE_URL: `https://${id}.invalid`, ANTHROPIC_AUTH_TOKEN: 'ci-dummy-not-a-secret',
      } } },
    });
    await invoke('switch_provider', { tool: 'claude', id });
    const live = JSON.parse(await readFile(join(home, '.claude', 'settings.json'), 'utf8'));
    assert.equal(live.env.ANTHROPIC_BASE_URL, `https://${id}.invalid`);
    assert.equal((await invoke('list_providers', { tool: 'claude' })).current, id);
  }
  for (let attempt = 0; attempt < 20; attempt++) {
    if ((await evaluate('document.body.innerText')).includes('windows-ci-b')) break;
    await delay(250);
  }
  assert.match(await evaluate('document.body.innerText'), /windows-ci-b/);
  const screenshot = await cdp('Page.captureScreenshot', { format: 'png' });
  await mkdir('target', { recursive: true });
  await writeFile('target/windows-smoke.png', Buffer.from(screenshot.data, 'base64'));
  console.log('Windows rendered UI, isolated paths, frontend IPC, two provider switches and disk readback passed');
} finally {
  socket?.close();
  clearTimeout(timeout);
}
