#!/usr/bin/env node
import { desktop, parseDesktopArgs } from '../lib/distribution.js';

try {
  await desktop(parseDesktopArgs(process.argv.slice(2)));
} catch (error) {
  console.error(`SwitchLeaf: ${error.message}`);
  process.exitCode = 1;
}
