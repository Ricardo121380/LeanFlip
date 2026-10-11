import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('release metadata preserves v0.1.0 assets and prepares only LeanFlip commands for new releases', async t => {
  const root = await mkdtemp(join(tmpdir(), 'leanflip-metadata-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const part of ['scripts', 'packages/npm', 'assets', 'docs/releases']) await mkdir(join(root, part), { recursive: true });
  await copyFile(new URL('../../../scripts/prepare-distribution.mjs', import.meta.url), join(root, 'scripts/prepare-distribution.mjs'));
  await copyFile(new URL('../package.json', import.meta.url), join(root, 'packages/npm/package.json'));
  await writeFile(join(root, 'LICENSE'), 'Current fixture license');
  await writeFile(join(root, 'docs/releases/v0.1.0-LICENSE'), 'Legacy fixture license');
  await writeFile(join(root, 'docs/releases/v0.1.0-NOTICE.md'), 'Legacy fixture notice');
  for (const version of ['0.1.0', '0.2.0', '0.1.0']) {
    const cliPrefix = version === '0.1.0' ? 'ccsl' : 'leanflip';
    const appPrefix = version === '0.1.0' ? 'ccswitch-lite' : 'leanflip';
    const names = [`${cliPrefix}_${version}_macos-arm64.tar.gz`, `${cliPrefix}_${version}_linux-x64.tar.gz`, `${appPrefix}_${version}_macos-arm64.dmg`, `${appPrefix}_${version}_linux-x64.deb`];
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
    if (version === '0.1.0') {
      assert.equal(await readFile(join(root, 'packages/npm/LICENSE'), 'utf8'), 'Legacy fixture license');
      assert.match(await readFile(join(root, 'packages/npm/NOTICE.md'), 'utf8'), /Legacy fixture notice/);
      assert.ok(pkg.files.includes('NOTICE.md'));
      assert.equal(pkg.name, 'ccswitch-lite'); assert.equal(manifest.appName, undefined);
      assert.equal(manifest.cliName, undefined);
      assert.deepEqual(Object.keys(pkg.bin).sort(), ['ccsl', 'ccsl-app']);
      assert.match(await readFile(join(root, 'Casks/ccswitch-lite.rb'), 'utf8'), /app "CC Switch Lite.app"/);
    } else {
      assert.equal(await readFile(join(root, 'packages/npm/LICENSE'), 'utf8'), 'Current fixture license');
      await assert.rejects(readFile(join(root, 'packages/npm/NOTICE.md')), { code: 'ENOENT' });
      assert.ok(!pkg.files.includes('NOTICE.md'));
      assert.equal(pkg.name, 'leanflip'); assert.equal(manifest.appName, 'LeanFlip.app');
      assert.equal(manifest.cliName, 'leanflip');
      assert.deepEqual(Object.keys(pkg.bin).sort(), ['leanflip','leanflip-app']);
      assert.match(await readFile(join(root, 'Casks/leanflip.rb'), 'utf8'), /binary "leanflip"/);
      assert.match(await readFile(join(root, 'Casks/leanflip-app.rb'), 'utf8'), /app "LeanFlip.app"/);
    }
  }
});

test('installers extract the named CLI and show the matching product without downloading', async t => {
  const root = await mkdtemp(join(tmpdir(), 'leanflip-installer-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const [version, name, product] of [['0.1.0', 'ccsl', 'CC Switch Lite'], ['0.2.0', 'leanflip', 'LeanFlip']]) {
    const fixture = join(root, version);
    await mkdir(join(fixture, 'lib'), { recursive: true });
    await mkdir(join(fixture, 'native'));
    await copyFile(new URL('../lib/distribution.js', import.meta.url), join(fixture, 'lib/distribution.js'));
    await writeFile(join(fixture, 'package.json'), '{"type":"module"}');
    await writeFile(join(fixture, 'native', name), `#!/bin/sh\nprintf '${name}-fixture\\n'\n`, { mode: 0o755 });
    const archive = `${name}.tar.gz`;
    execFileSync('tar', ['-czf', join(fixture, archive), '-C', join(fixture, 'native'), name]);
    const digest = createHash('sha256').update(await readFile(join(fixture, archive))).digest('hex');
    const cache = join(fixture, 'cache', 'ccswitch-lite', version, digest);
    await mkdir(cache, { recursive: true });
    await copyFile(join(fixture, archive), join(cache, archive));
    const asset = { name: archive, sha256: digest, url: 'https://example.invalid/must-use-verified-cache' };
    await writeFile(join(fixture, 'release.json'), JSON.stringify({
      version, homepage: 'https://example.invalid',
      ...(name === 'leanflip' ? { cliName: name, appName: `${product}.app` } : {}),
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
