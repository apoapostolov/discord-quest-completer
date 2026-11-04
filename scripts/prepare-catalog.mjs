// Reproduce the upstream bundled fallback from an immutable, checksummed snapshot.
import { createHash } from 'node:crypto';
import { mkdir, writeFile } from 'node:fs/promises';

const revision = '53191138e647304dc7dce6a6ace77ae19f8309c3';
const expected = '04c2754d85eabe06232426931fb55e8f0cd553689735aa3d8def32f6e5a425d4';
const url = `https://raw.githubusercontent.com/markterence/discord-quest-completer/${revision}/src/assets/gamelist.json`;
const response = await fetch(url, { signal: AbortSignal.timeout(30000) });
if (!response.ok) throw new Error(`Catalog download failed: HTTP ${response.status}`);
const data = Buffer.from(await response.arrayBuffer());
if (createHash('sha256').update(data).digest('hex') !== expected) throw new Error('Catalog checksum mismatch');
const catalog = JSON.parse(data.toString('utf8'));
if (!Array.isArray(catalog) || !catalog.length || !catalog.every(item =>
  item && typeof item.id === 'string' && typeof item.name === 'string' &&
  Array.isArray(item.aliases) && item.aliases.every(alias => typeof alias === 'string') &&
  Array.isArray(item.executables) && item.executables.every(executable => executable &&
    typeof executable.name === 'string' && typeof executable.os === 'string' &&
    typeof executable.is_launcher === 'boolean')))
  throw new Error('Pinned catalog has an invalid schema');
const target = new URL('../src/assets/gamelist.json', import.meta.url);
await mkdir(new URL('../src/assets/', import.meta.url), { recursive: true });
await writeFile(target, data);
console.log(`Prepared ${catalog.length} catalog records from upstream ${revision}; SHA-256 verified.`);
