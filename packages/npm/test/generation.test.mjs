import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, writeFile, copyFile, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('release metadata names the LeanFlip commands and app and drops files from earlier releases', async t => {
  const root = await mkdtemp(join(tmpdir(), 'leanflip-metadata-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const part of ['scripts', 'packages/npm', 'assets', 'crates/app', 'Casks']) await mkdir(join(root, part), { recursive: true });
  await copyFile(new URL('../../../scripts/prepare-distribution.mjs', import.meta.url), join(root, 'scripts/prepare-distribution.mjs'));
  await copyFile(new URL('../../../crates/app/tauri.conf.json', import.meta.url), join(root, 'crates/app/tauri.conf.json'));
  await copyFile(new URL('../package.json', import.meta.url), join(root, 'packages/npm/package.json'));
  await writeFile(join(root, 'LICENSE'), 'Current fixture license');
  await writeFile(join(root, 'packages/npm/NOTICE.md'), 'Notice from an earlier release');
  await writeFile(join(root, 'Casks/earlier.rb'), 'cask "earlier" do\nend\n');
  const tauri = JSON.parse(await readFile(join(root, 'crates/app/tauri.conf.json'), 'utf8'));
  for (const version of ['0.2.0', '0.2.1']) {
    const names = ['macos-arm64.tar.gz', 'linux-x64.tar.gz', 'macos-arm64.dmg', 'linux-x64.deb'].map(suffix => `leanflip_${version}_${suffix}`);
    const sums = [];
    for (const name of names) {
      const bytes = Buffer.from(`Metadata-only fixture ${name}`);
      await writeFile(join(root, 'assets', name), bytes);
      sums.push(`${createHash('sha256').update(bytes).digest('hex')}  ${name}`);
    }
    await writeFile(join(root, 'assets/SHA256SUMS'), sums.join('\n') + '\n');
    const args = [join(root, 'scripts/prepare-distribution.mjs'), version, join(root, 'assets')];
    execFileSync(process.execPath, args); execFileSync(process.execPath, [...args, '--check']);
    const pkg = JSON.parse(await readFile(join(root, 'packages/npm/package.json')));
    const manifest = JSON.parse(await readFile(join(root, 'packages/npm/release.json')));
    assert.equal(await readFile(join(root, 'packages/npm/LICENSE'), 'utf8'), 'Current fixture license');
    await assert.rejects(readFile(join(root, 'packages/npm/NOTICE.md')), { code: 'ENOENT' });
    assert.ok(!pkg.files.includes('NOTICE.md'));
    assert.equal(pkg.name, 'leanflip');
    assert.deepEqual(pkg.bin, { leanflip: 'bin/leanflip.js', 'leanflip-app': 'bin/leanflip-app.js' });
    assert.deepEqual([manifest.appName, manifest.cliName, manifest.appCask], ['LeanFlip.app', 'leanflip', 'leanflip-app']);
    assert.deepEqual([manifest.appBinary, manifest.bundleId], [tauri.mainBinaryName, tauri.identifier]);
    assert.deepEqual((await readdir(join(root, 'Casks'))).sort(), ['leanflip-app.rb', 'leanflip.rb']);
    assert.match(await readFile(join(root, 'Casks/leanflip.rb'), 'utf8'), /binary "leanflip"/);
    const app = await readFile(join(root, 'Casks/leanflip-app.rb'), 'utf8');
    assert.match(app, /app "LeanFlip.app"/);
    assert.ok(app.includes(`uninstall quit: "${tauri.identifier}"`));
  }
  // A stale earlier Cask makes the check fail instead of being published.
  await writeFile(join(root, 'Casks/earlier.rb'), 'cask "earlier" do\nend\n');
  assert.throws(() => execFileSync(process.execPath, [join(root, 'scripts/prepare-distribution.mjs'), '0.2.1', join(root, 'assets'), '--check'], { stdio: 'pipe' }));
  assert.throws(() => execFileSync(process.execPath, [join(root, 'scripts/prepare-distribution.mjs'), '0.1.0', join(root, 'assets')], { stdio: 'pipe' }));
});

test('installers extract the named CLI and show the matching product without downloading', async t => {
  const root = await mkdtemp(join(tmpdir(), 'leanflip-installer-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [version, name, product] of [['0.1.0', 'legacy', 'Earlier Name'], ['0.2.0', 'leanflip', 'LeanFlip']]) {
    const fixture = join(root, version);
    await mkdir(join(fixture, 'lib'), { recursive: true });
    await mkdir(join(fixture, 'native'));
    await copyFile(new URL('../lib/distribution.js', import.meta.url), join(fixture, 'lib/distribution.js'));
    await writeFile(join(fixture, 'package.json'), '{"type":"module"}');
    await writeFile(join(fixture, 'native', name), `#!/bin/sh\nprintf '${name}-fixture\\n'\n`, { mode: 0o755 });
    const archive = `${name}.tar.gz`;
    execFileSync('tar', ['-czf', join(fixture, archive), '-C', join(fixture, 'native'), name]);
    const digest = createHash('sha256').update(await readFile(join(fixture, archive))).digest('hex');
    const cache = join(fixture, 'cache', 'leanflip', version, digest);
    await mkdir(cache, { recursive: true });
    await copyFile(join(fixture, archive), join(cache, archive));
    const asset = { name: archive, sha256: digest, url: 'https://example.invalid/must-use-verified-cache' };
    await writeFile(join(fixture, 'release.json'), JSON.stringify({
      version, homepage: 'https://example.invalid',
      cliName: name, appName: `${product}.app`, appBinary: `${name}-app`, appCask: `${name}-app`, bundleId: `dev.${name}.desktop`,
      platforms: { [`${process.platform}-${process.arch}`]: { cli: asset, app: asset } },
    }));
    const output = execFileSync(process.execPath, ['--input-type=module', '-e',
      "import { ensureCli, command, desktop } from './lib/distribution.js'; console.log(command(await ensureCli(), [])); await desktop({ action: 'help' });",
    ], { cwd: fixture, env: { ...process.env, XDG_CACHE_HOME: join(fixture, 'cache') }, encoding: 'utf8' });
    assert.match(output, new RegExp(`${name}-fixture`));
    assert.ok(output.includes(`${product} ${version}`));
    assert.ok(output.includes(`${name}-app install`));
    assert.equal(await readFile(join(cache, name), 'utf8'), `#!/bin/sh\nprintf '${name}-fixture\\n'\n`);
  }
});
