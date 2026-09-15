#!/usr/bin/env node
// Checks that every locale in dist/js/locales has exactly the keys of en.js.
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const localesDir = join(dirname(fileURLToPath(import.meta.url)), '..', 'dist', 'js', 'locales');
const REFERENCE = 'en.js';
const KEY_RE = /^\s*(['"])([^'"]+)\1\s*:/;

function keysOf(file) {
  const keys = new Set();
  const text = readFileSync(join(localesDir, file), 'utf8');
  for (const line of text.split(/\r?\n/)) {
    const m = KEY_RE.exec(line);
    if (m) keys.add(m[2]);
  }
  return keys;
}

const refKeys = keysOf(REFERENCE);
const files = readdirSync(localesDir)
  .filter((f) => f.endsWith('.js') && f !== REFERENCE)
  .sort();

let failed = false;
for (const file of files) {
  const keys = keysOf(file);
  const missing = [...refKeys].filter((k) => !keys.has(k));
  const extra = [...keys].filter((k) => !refKeys.has(k));
  if (missing.length || extra.length) {
    failed = true;
    console.error(`✗ ${file}`);
    if (missing.length) console.error(`    missing (${missing.length}): ${missing.join(', ')}`);
    if (extra.length) console.error(`    extra (${extra.length}): ${extra.join(', ')}`);
  } else {
    console.log(`✓ ${file} (${keys.size} keys)`);
  }
}

if (failed) {
  console.error(`\ni18n check FAILED: some languages differ from ${REFERENCE}.`);
  process.exit(1);
}
console.log(`\ni18n check OK: ${files.length} languages match ${REFERENCE} (${refKeys.size} keys).`);
