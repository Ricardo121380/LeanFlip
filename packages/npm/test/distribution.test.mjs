import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { linuxInstallCommand, parseDesktopArgs, selectAsset, validateExistingApp, verifiedDownload } from '../lib/distribution.js';

async function temporary(t) {
  const dir = await mkdtemp(join(tmpdir(), 'ccsl-distribution-test-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return dir;
}
function asset(body) {
  return { name: 'package.tar.gz', url: 'https://example.com/release/package.tar.gz', sha256: createHash('sha256').update(body).digest('hex') };
}

test('chooses the matching release and rejects unsupported OS/CPU pairs', () => {
  assert.match(selectAsset('cli', 'darwin', 'arm64').name, /macos-arm64\.tar\.gz$/);
  assert.match(selectAsset('app', 'linux', 'x64').name, /linux-x64\.deb$/);
  for (const [platform, arch] of [['darwin', 'x64'], ['linux', 'arm64'], ['win32', 'x64']]) {
    assert.throws(() => selectAsset('cli', platform, arch), /Unsupported platform/);
  }
});

test('verifies downloads, reuses a valid cache and repairs corruption', async t => {
  const dir = await temporary(t); const bytes = Buffer.from('verified archive'); const release = asset(bytes);
  let calls = 0;
  const fetcher = async () => { calls++; return new Response(bytes); };
  const path = await verifiedDownload(release, dir, fetcher);
  assert.deepEqual(await readFile(path), bytes);
  await verifiedDownload(release, dir, fetcher);
  assert.equal(calls, 1);
  await writeFile(path, 'corrupted cache');
  await verifiedDownload(release, dir, fetcher);
  assert.equal(calls, 2);
  assert.deepEqual(await readFile(path), bytes);
});

test('refuses mismatched SHA256 without replacing an existing download', async t => {
  const dir = await temporary(t); const release = asset('expected');
  await writeFile(join(dir, release.name), 'previous download');
  await assert.rejects(verifiedDownload(release, dir, async () => new Response('tampered')), /SHA256 mismatch/);
  assert.equal(await readFile(join(dir, release.name), 'utf8'), 'previous download');
  assert.deepEqual(await readdir(dir), [release.name]);
});

test('HTTP and interrupted stream failures leave no partial files', async t => {
  const dir = await temporary(t); const release = asset('expected');
  await assert.rejects(verifiedDownload(release, dir, async () => new Response('', { status: 404 })), /HTTP 404/);
  await assert.rejects(verifiedDownload(release, dir, async () => new Response(new ReadableStream({
    start(controller) { controller.enqueue(new Uint8Array([1])); controller.error(new Error('connection lost')); },
  }))), /connection lost/);
  assert.deepEqual(await readdir(dir), []);
});

test('rejects paths and insecure transport before writing a download', async t => {
  const dir = await temporary(t); const release = asset('expected');
  const fetcher = () => { throw new Error('must not fetch'); };
  for (const invalid of [{ name: '../escape' }, { sha256: 'invalid' }, { url: 'http://example.com/file' }]) {
    await assert.rejects(verifiedDownload({ ...release, ...invalid }, dir, fetcher), /Invalid release metadata/);
  }
  await assert.rejects(verifiedDownload(release, dir, async () => {
    const response = new Response('expected');
    Object.defineProperty(response, 'url', { value: 'http://example.com/file' });
    return response;
  }), /insecure download redirect/);
  assert.deepEqual(await readdir(dir), []);
});

test('rejects oversized responses and cleans the temporary download', async t => {
  const dir = await temporary(t); let remaining = 129;
  const fetcher = async () => new Response(new ReadableStream({ pull(controller) {
    if (remaining--) controller.enqueue(new Uint8Array(1024 * 1024)); else controller.close();
  } }));
  await assert.rejects(verifiedDownload(asset('expected'), dir, fetcher), /128 MiB/);
  assert.deepEqual(await readdir(dir), []);
});

test('app replacement requires an explicit flag and the correct bundle identity', async t => {
  const dir = await temporary(t); const path = join(dir, 'App.app');
  assert.equal(await validateExistingApp(path, false, () => 'dev.ccswitch.lite'), false);
  await mkdir(path);
  await assert.rejects(validateExistingApp(path, false, () => 'dev.ccswitch.lite'), /--replace/);
  assert.equal(await validateExistingApp(path, true, () => 'dev.ccswitch.lite'), true);
  await assert.rejects(validateExistingApp(path, true, () => 'other.app'), /unrelated/);
  await symlink(path, join(dir, 'Link.app'));
  await assert.rejects(validateExistingApp(join(dir, 'Link.app'), true, () => 'dev.ccswitch.lite'), /symlink/);
});

test('desktop arguments reject unknown options and missing directory values', () => {
  assert.equal(parseDesktopArgs([]).action, 'help');
  assert.equal(parseDesktopArgs(['--version']).action, 'version');
  assert.deepEqual(parseDesktopArgs(['install', '--app-dir', '/tmp/Apps', '--replace']), { action: 'install', appDir: '/tmp/Apps', replace: true });
  for (const args of [['unknown'], ['install', '--output', '.'], ['install', '--app-dir'], ['install', '--app-dir', 'relative'], ['download', '--output'], ['open', '--replace']]) {
    assert.throws(() => parseDesktopArgs(args));
  }
});

test('Linux delegates package installation to apt without invoking a shell', () => {
  const file = '/tmp/a directory/app.deb';
  assert.deepEqual(linuxInstallCommand(file, 1000), ['sudo', ['apt', 'install', file]]);
  assert.deepEqual(linuxInstallCommand(file, 0), ['apt', ['install', file]]);
});
