import { createHash, randomUUID } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { createReadStream, createWriteStream } from 'node:fs';
import { chmod, lstat, mkdir, mkdtemp, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { pipeline } from 'node:stream/promises';
import { Readable, Transform } from 'node:stream';

export const manifest = JSON.parse(await readFile(new URL('../release.json', import.meta.url), 'utf8'));
export const appName = manifest.appName ?? 'CC Switch Lite.app';
export const productName = appName.replace(/\.app$/, '');
export const cliName = manifest.cliName ?? 'ccsl';
const desktopCommand = `${cliName}-app`;
const bundleId = 'dev.ccswitch.lite';
const maxDownload = 128 * 1024 * 1024;

export function selectAsset(kind, platform = process.platform, arch = process.arch) {
  const asset = manifest.platforms[`${platform}-${arch}`]?.[kind];
  if (!asset) throw new Error(`Unsupported platform: ${platform}-${arch}. This release supports macOS ARM64 and Linux x64. See ${manifest.homepage}/releases.`);
  return asset;
}

export function cacheRoot() {
  const base = process.env.XDG_CACHE_HOME;
  return join(base && isAbsolute(base) ? base : join(homedir(), '.cache'), 'ccswitch-lite', manifest.version);
}

export async function sha256(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

async function regularFile(path) {
  try { return (await lstat(path)).isFile(); }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

export async function verifiedDownload(asset, directory, fetcher = fetch) {
  if (!/^[a-f0-9]{64}$/.test(asset.sha256) || basename(asset.name) !== asset.name || !asset.url.startsWith('https://')) {
    throw new Error('Invalid release metadata. Reinstall the npm package.');
  }
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const target = join(directory, asset.name);
  if (await regularFile(target) && await sha256(target) === asset.sha256) return target;
  const partial = join(directory, `.${asset.name}.${randomUUID()}.part`);
  try {
    const response = await fetcher(asset.url, { signal: AbortSignal.timeout(60_000) });
    if (!response.ok || !response.body) throw new Error(`Download failed (HTTP ${response.status}): ${asset.name}`);
    if (response.url && !response.url.startsWith('https://')) throw new Error('Refusing an insecure download redirect.');
    let received = 0;
    const limit = new Transform({ transform(chunk, _encoding, done) {
      received += chunk.length;
      done(received > maxDownload ? new Error('Download exceeds the 128 MiB limit.') : null, chunk);
    } });
    await pipeline(Readable.fromWeb(response.body), limit, createWriteStream(partial, { flags: 'wx', mode: 0o600 }));
    if (await sha256(partial) !== asset.sha256) throw new Error(`SHA256 mismatch: ${asset.name}. The file was not installed.`);
    await rename(partial, target);
    return target;
  } finally {
    await rm(partial, { force: true });
  }
}

export function command(program, args, options = {}) {
  const result = spawnSync(program, args, { encoding: 'utf8', ...options });
  if (result.error) throw new Error(`Cannot run ${program}: ${result.error.code ?? result.error.message}`);
  if (result.status !== 0) throw new Error(`${program} failed (${result.signal ?? result.status}). Check its output and directory permissions.`);
  return result.stdout?.trim() ?? '';
}

export async function ensureCli() {
  const asset = selectAsset('cli');
  const directory = join(cacheRoot(), asset.sha256);
  const binary = join(directory, cliName);
  const receipt = join(directory, `${cliName}.sha256`);
  if (await regularFile(binary) && await regularFile(receipt) && await sha256(binary) === (await readFile(receipt, 'utf8')).trim()) return binary;
  const archive = await verifiedDownload(asset, directory);
  const staging = await mkdtemp(join(directory, '.extract-'));
  try {
    // Extract only the expected member; never execute archive-provided scripts.
    command('tar', ['-xzf', archive, '-C', staging, cliName]);
    const extracted = join(staging, cliName);
    const stat = await lstat(extracted);
    if (!stat.isFile() || stat.nlink !== 1) throw new Error(`The CLI archive does not contain a regular ${cliName} executable.`);
    await chmod(extracted, 0o755);
    const digest = await sha256(extracted);
    await rename(extracted, binary);
    await writeFile(receipt, digest, { mode: 0o600 });
    return binary;
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

export function parseDesktopArgs(args) {
  const options = { action: args[0] ?? 'help', replace: false };
  if (options.action === '--help' || options.action === '-h') options.action = 'help';
  if (options.action === '--version' || options.action === '-V') options.action = 'version';
  if (!['help', 'version', 'install', 'download', 'open', 'path'].includes(options.action)) throw new Error(`Unknown command. Run ${desktopCommand} --help.`);
  for (let i = 1; i < args.length; i++) {
    if (args[i] === '--replace' && options.action === 'install') options.replace = true;
    else if (args[i] === '--app-dir' && ['install', 'open', 'path'].includes(options.action)) {
      if (!args[++i] || !isAbsolute(args[i])) throw new Error('--app-dir requires an absolute directory.');
      options.appDir = args[i];
    } else if (args[i] === '--output' && options.action === 'download') {
      if (!args[++i]) throw new Error('--output requires a directory.');
      options.output = resolve(args[i]);
    } else throw new Error(`Invalid option. Run ${desktopCommand} --help.`);
  }
  return options;
}

export function macAppPath(appDir) {
  return join(appDir ?? join(homedir(), 'Applications'), appName);
}

async function exists(path) {
  try { await lstat(path); return true; }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

export async function validateExistingApp(target, replace, inspectBundle) {
  if (!await exists(target)) return false;
  const stat = await lstat(target);
  if (!stat.isDirectory() || stat.isSymbolicLink() || inspectBundle(target) !== bundleId) {
    throw new Error(`Refusing to replace an unrelated app or symlink: ${target}`);
  }
  if (!replace) throw new Error(`Already installed: ${target}. Quit the app, then use install --replace to update it.`);
  return true;
}

function inspectBundle(path) {
  return command('/usr/libexec/PlistBuddy', ['-c', 'Print :CFBundleIdentifier', join(path, 'Contents', 'Info.plist')]);
}

export async function installMacApp(archive, options) {
  const target = macAppPath(options.appDir);
  const hadExisting = await validateExistingApp(target, options.replace, inspectBundle);
  if (hadExisting && spawnSync('/usr/bin/pgrep', ['-x', 'ccswitch-lite']).status === 0) {
    throw new Error(`Quit ${productName} before replacing the installed application.`);
  }
  const work = await mkdtemp(join(tmpdir(), 'ccswitch-lite-mount-'));
  const mount = join(work, 'volume');
  await mkdir(mount);
  let mounted = false;
  let staging;
  let backup;
  try {
    command('/usr/bin/hdiutil', ['attach', archive, '-readonly', '-nobrowse', '-mountpoint', mount]);
    mounted = true;
    const source = join(mount, appName);
    if (inspectBundle(source) !== bundleId) throw new Error('Unexpected application in the release image.');
    await mkdir(dirname(target), { recursive: true });
    staging = await mkdtemp(join(dirname(target), '.ccswitch-lite-install-'));
    const prepared = join(staging, appName);
    command('/usr/bin/ditto', [source, prepared]);
    if (inspectBundle(prepared) !== bundleId) throw new Error('Copied application failed validation.');
    if (await exists(target)) {
      // Check again after downloading/copying, so concurrent installs cannot overwrite a new app.
      if (!hadExisting) throw new Error('An app appeared at the destination during installation. Please retry.');
      await validateExistingApp(target, options.replace, inspectBundle);
      backup = join(staging, 'previous.app');
      await rename(target, backup);
    }
    try { await rename(prepared, target); }
    catch (error) {
      if (backup) { await rename(backup, target); backup = undefined; }
      throw error;
    }
    if (backup) { await rm(backup, { recursive: true }); backup = undefined; }
    return target;
  } finally {
    if (mounted) command('/usr/bin/hdiutil', ['detach', mount]);
    await rm(work, { recursive: true, force: true });
    // Keep a recovery copy if restoring an existing application failed.
    if (staging && !backup) await rm(staging, { recursive: true, force: true });
  }
}

export function linuxInstallCommand(archive, uid = process.getuid?.()) {
  return uid === 0 ? ['apt', ['install', archive]] : ['sudo', ['apt', 'install', archive]];
}

export async function desktop(options) {
  if (options.action === 'version') { console.log(`${desktopCommand} ${manifest.version}`); return; }
  if (options.action === 'help') {
    console.log(`${productName} ${manifest.version} · 桌面安装器 / Desktop installer

用法 / Usage:
  ${desktopCommand} install                 安装桌面 App / Install the desktop app
  ${desktopCommand} install --replace       更新已有 App / Replace a previous install
  ${desktopCommand} download --output .     只下载并校验 / Download and verify only
  ${desktopCommand} open                    启动 App / Open the app
  ${desktopCommand} path                    显示默认安装位置 / Show the app path
  ${cliName}                               打开原生 CLI 菜单 / Open the native CLI menu

macOS ARM64: 默认安装到 ~/Applications，--app-dir 可指定绝对路径。
Linux x64: install 使用 apt（普通用户通过 sudo）；系统会提示确认和密码。
App 尚未做 Apple Developer ID 签名及公证。安装不会自动启动 App。
文档 / Documentation: ${manifest.homepage}#readme`);
    return;
  }
  const asset = selectAsset('app');
  if (process.platform !== 'darwin' && options.appDir) throw new Error('--app-dir is only supported on macOS.');
  if (options.action === 'path') {
    console.log(process.platform === 'darwin' ? macAppPath(options.appDir) : '/usr/bin/ccswitch-lite');
    return;
  }
  if (options.action === 'open') {
    if (process.platform === 'darwin') {
      let target = macAppPath(options.appDir);
      if (!options.appDir && !await exists(target)) target = join('/Applications', appName);
      if (!await exists(target) || inspectBundle(target) !== bundleId) throw new Error(`App not found. Run ${desktopCommand} install first.`);
      command('/usr/bin/open', [target], { stdio: 'inherit' });
    } else command('/usr/bin/ccswitch-lite', [], { stdio: 'inherit' });
    return;
  }
  const directory = options.output ?? join(cacheRoot(), asset.sha256);
  console.error(`下载并校验 / Downloading and verifying ${asset.name}`);
  const archive = await verifiedDownload(asset, directory);
  if (options.action === 'download') { console.log(archive); return; }
  if (process.platform === 'darwin') {
    const target = await installMacApp(archive, options);
    console.log(`已安装 / Installed: ${target}\n启动 / Open: ${desktopCommand} open`);
  } else {
    if (!process.stdin.isTTY) throw new Error(`A terminal is required for apt installation. Verified package: ${archive}\nRun: sudo apt install '${archive.replaceAll("'", "'\\''")}'`);
    const [program, args] = linuxInstallCommand(archive);
    command(program, args, { stdio: 'inherit' });
    console.log(`已安装 / Installed: ${productName}. Launch it from your desktop application menu.`);
  }
}
