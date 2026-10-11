#!/usr/bin/env node
import { desktop, parseDesktopArgs, productName } from '../lib/distribution.js';

try {
  await desktop(parseDesktopArgs(process.argv.slice(2)));
} catch (error) {
  console.error(`${productName}: ${error.message}`);
  process.exitCode = 1;
}
