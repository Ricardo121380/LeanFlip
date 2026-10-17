#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { cliName, ensureCli } from '../lib/distribution.js';

try {
  const binary = await ensureCli();
  const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
  // Stay alive until the native menu restores the terminal and reports its exit.
  // Forward signals sent directly to the wrapper as well as terminal interrupts.
  const handlers = new Map(['SIGINT', 'SIGTERM', 'SIGHUP'].map(signal => [signal, () => {
    // Windows broadcasts console Ctrl+C to both processes. child.kill('SIGINT')
    // forcibly terminates the child there, preventing its terminal cleanup.
    if (process.platform !== 'win32' || signal !== 'SIGINT') child.kill(signal);
  }]));
  for (const [signal, handler] of handlers) process.on(signal, handler);
  let result;
  try {
    result = await new Promise((resolve, reject) => {
      child.once('error', error => reject(new Error(`Cannot start the native CLI: ${error.code ?? error.message}`)));
      child.once('close', (code, signal) => resolve({ code, signal }));
    });
  } finally {
    for (const [signal, handler] of handlers) process.off(signal, handler);
  }
  if (result.signal) process.kill(process.pid, result.signal);
  else process.exitCode = result.code ?? 1;
} catch (error) {
  const message = `Cannot prepare ${cliName}: ${error.message}`;
  console.error(process.argv.includes('--json')
    ? JSON.stringify({ ok: false, error: { code: 'binary_install_failed', message } })
    : message);
  process.exitCode = 1;
}
