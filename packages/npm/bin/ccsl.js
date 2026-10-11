#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { ensureCli } from '../lib/distribution.js';

try {
  const binary = await ensureCli();
  const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });
  if (result.error) throw new Error(`Cannot start the native CLI: ${result.error.code ?? result.error.message}`);
  if (result.signal) process.kill(process.pid, result.signal);
  else process.exitCode = result.status ?? 1;
} catch (error) {
  const message = `Cannot prepare ccsl: ${error.message}`;
  console.error(process.argv.includes('--json')
    ? JSON.stringify({ ok: false, error: { code: 'binary_install_failed', message } })
    : message);
  process.exitCode = 1;
}
