import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, copyFile, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('release metadata preserves v0.1.0 names and prepares SwitchLeaf with compatibility aliases', async t => {
  const root = await mkdtemp(join(tmpdir(), 'switchleaf-metadata-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  for (const part of ['scripts', 'packages/npm', 'assets']) await mkdir(join(root, part), { recursive: true });
  await copyFile(new URL('../../../scripts/prepare-distribution.mjs', import.meta.url), join(root, 'scripts/prepare-distribution.mjs'));
  await copyFile(new URL('../package.json', import.meta.url), join(root, 'packages/npm/package.json'));
  await writeFile(join(root, 'LICENSE'), 'Fixture license'); await writeFile(join(root, 'NOTICE.md'), 'Fixture notice');
  for (const version of ['0.1.0', '0.2.0']) {
    const names = [`ccsl_${version}_macos-arm64.tar.gz`, `ccsl_${version}_linux-x64.tar.gz`, `ccswitch-lite_${version}_macos-arm64.dmg`, `ccswitch-lite_${version}_linux-x64.deb`];
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
      assert.equal(pkg.name, 'ccswitch-lite'); assert.equal(manifest.appName, undefined);
      assert.match(await readFile(join(root, 'Casks/ccswitch-lite.rb'), 'utf8'), /app "CC Switch Lite.app"/);
    } else {
      assert.equal(pkg.name, 'switchleaf'); assert.equal(manifest.appName, 'SwitchLeaf.app');
      assert.deepEqual(Object.keys(pkg.bin).sort(), ['ccsl','ccsl-app','switchleaf','switchleaf-app']);
      assert.match(await readFile(join(root, 'Casks/switchleaf.rb'), 'utf8'), /binary "switchleaf"/);
      assert.match(await readFile(join(root, 'Casks/switchleaf-app.rb'), 'utf8'), /app "SwitchLeaf.app"/);
    }
  }
});
