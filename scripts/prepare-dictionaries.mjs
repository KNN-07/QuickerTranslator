#!/usr/bin/env node
/**
 * Prepare complete, licensed offline dictionary subsets. Node >= 22; no npm or
 * external unzip dependency. Inputs live outside the repository, never in the
 * application bundle. Ordinary execution verifies/reuses committed outputs;
 * --refresh is the only operation that accepts changed upstream snapshots.
 */
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { access, mkdir, mkdtemp, open, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { Readable, Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';
import { createGunzip, createInflateRaw } from 'node:zlib';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const OUT = join(ROOT, 'src-tauri', 'resources', 'dictionaries');
const MANIFEST = join(OUT, 'manifest.json');
const SCRIPT_VERSION = 3;
const SOURCES = {
  wiktextract: { url: 'https://kaikki.org/viwiktionary/raw-wiktextract-data.jsonl.gz', extension: '.jsonl.gz' },
  wiktextractMetadata: { url: 'https://kaikki.org/viwiktionary/rawdata.html', extension: '.html' },
  unihan: { url: 'https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip', extension: '.zip' },
};
const NOTICES = [
  { id: 'ipadic', file: 'licenses/IPADIC.txt', license: 'NAIST-ICOT', url: 'https://raw.githubusercontent.com/lindera/mecab-ipadic/master/COPYING' },
  { id: 'lindera', file: 'licenses/Lindera-MIT.txt', license: 'MIT', url: 'https://raw.githubusercontent.com/lindera/lindera/main/LICENSE' },
  { id: 'unicode', file: 'licenses/Unicode-V3.txt', license: 'Unicode-3.0', url: 'https://www.unicode.org/license.txt' },
  { id: 'wiktionary', file: 'licenses/CC-BY-SA-4.0.txt', license: 'CC-BY-SA-4.0', url: 'https://creativecommons.org/licenses/by-sa/4.0/legalcode.txt' },
];
const DATA_FILES = ['zh-vi.jsonl', 'ja-vi.jsonl', 'han-viet.jsonl'];
const IPADIC_ORIGINAL = 'licenses/IPADIC-original.COPYING';
const NOTICE_FILES = [...NOTICES.map((notice) => notice.file), IPADIC_ORIGINAL, 'ATTRIBUTION.txt'];
const orderedUnique = (values) => [...new Set(values)];
// Code-unit ordering is locale-independent and identical on every build host.
const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
const strings = (values) => Array.isArray(values) ? values.filter((v) => typeof v === 'string' && v.trim().length > 0) : [];
const pageUrl = (word) => `https://vi.wiktionary.org/wiki/${encodeURIComponent(word)}`;

function args() {
  const result = { refresh: false, verify: false, cache: resolve(process.env.QUICKTRANSLATOR_DATA_CACHE || join(homedir(), '.cache', 'quickertranslator-dictionaries')) };
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--refresh') result.refresh = true;
    else if (arg === '--verify') result.verify = true;
    else if (['--wiktextract', '--unihan', '--cache'].includes(arg)) {
      if (!argv[i + 1] || argv[i + 1].startsWith('--')) throw new Error(`Missing path after ${arg}`);
      result[arg.slice(2)] = resolve(argv[++i]);
    } else if (arg === '--help') {
      console.log(`Usage: npm run data:prepare -- [--verify | --refresh] [--wiktextract PATH] [--unihan PATH] [--cache PATH]\n\nDefault: verify and reuse committed resources without network access. If outputs\nare missing, regenerate using checksum-locked inputs from the cache or upstream.\n--verify: read-only offline validation of every output hash, entry schema/count,\n           ordering and license; missing resources are an error, never downloaded.\n--refresh: explicitly acquire a new snapshot and replace checksums/resources.\n--wiktextract: local copy of raw-wiktextract-data.jsonl.gz (not English data).\n--unihan: local copy of the complete official Unihan.zip.\nLocal inputs must match the manifest unless --refresh is supplied. Raw inputs are\nnot bundled. Cache defaults to ~/.cache/quickertranslator-dictionaries, overridden\nby QUICKTRANSLATOR_DATA_CACHE or --cache. Commit generated JSONL, manifest and\nlicense notices together. Installed applications never download dictionaries.`);
      process.exit(0);
    } else throw new Error(`Unknown argument: ${arg}`);
  }
  if (result.refresh && result.verify) throw new Error('--refresh and --verify are mutually exclusive');
  return result;
}

async function exists(path) {
  try { await access(path); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

async function digest(path) {
  const hash = createHash('sha256');
  let bytes = 0;
  for await (const chunk of createReadStream(path)) { hash.update(chunk); bytes += chunk.length; }
  return { sha256: hash.digest('hex'), bytes };
}

function requireDigest(observed, expected, label) {
  if (observed.sha256 !== expected.sha256 || observed.bytes !== expected.bytes) {
    throw new Error(`${label}: checksum/size differs from the locked manifest. Expected ${expected.sha256} (${expected.bytes} bytes), got ${observed.sha256} (${observed.bytes} bytes). Restore the locked snapshot or intentionally run --refresh; no updated corpus was accepted.`);
  }
}

async function loadManifest() {
  if (!await exists(MANIFEST)) return null;
  let value;
  try { value = JSON.parse(await readFile(MANIFEST, 'utf8')); } catch (error) { throw new Error(`Cannot read existing manifest: ${error.message}. Preserve it and restore a valid manifest, or explicitly use --refresh.`); }
  if (value.format !== 'quicktranslator-dictionaries' || value.version !== 1 || !value.sources || !Array.isArray(value.resources)) {
    throw new Error('Unsupported dictionary manifest. Restore version 1 or explicitly use --refresh.');
  }
  return value;
}

async function acquire(id, specification, options, locked, localPath) {
  const expected = options.refresh ? null : locked;
  if (localPath) {
    const observed = await digest(localPath);
    if (expected) requireDigest(observed, expected, `Local ${id}`);
    return { path: localPath, metadata: expected || { ...observed, url: specification.url, retrievedAt: new Date().toISOString(), suppliedLocally: true, lastModified: null } };
  }
  if (expected) {
    const cached = join(options.cache, `${id}-${expected.sha256}${specification.extension || '.txt'}`);
    if (await exists(cached)) {
      requireDigest(await digest(cached), expected, `Cached ${id}`);
      return { path: cached, metadata: expected };
    }
  }
  await mkdir(options.cache, { recursive: true });
  const temporary = join(options.cache, `${id}-${randomUUID()}.partial`);
  console.error(`Acquiring ${id}: ${specification.url}`);
  try {
    const response = await fetch(specification.url, { signal: AbortSignal.timeout(300_000), headers: { 'User-Agent': 'QuickTranslator-offline-data-preparation/1' } });
    if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`);
    const hash = createHash('sha256');
    let bytes = 0;
    const meter = new Transform({ transform(chunk, _encoding, callback) { hash.update(chunk); bytes += chunk.length; callback(null, chunk); } });
    await pipeline(Readable.fromWeb(response.body), meter, createWriteStream(temporary, { flags: 'wx' }));
    const observed = { sha256: hash.digest('hex'), bytes };
    if (expected) requireDigest(observed, expected, `Downloaded ${id}`);
    const destination = join(options.cache, `${id}-${observed.sha256}${specification.extension || '.txt'}`);
    if (await exists(destination)) await rm(temporary); else await rename(temporary, destination);
    return { path: destination, metadata: expected || { ...observed, url: specification.url, retrievedAt: new Date().toISOString(), suppliedLocally: false, lastModified: response.headers.get('last-modified') } };
  } catch (error) {
    await rm(temporary, { force: true });
    const instruction = id === 'wiktextract' ? ' Supply the same Vietnamese raw JSONL gzip using --wiktextract PATH.' : id === 'unihan' ? ' Supply the complete official archive using --unihan PATH.' : '';
    throw new Error(`Acquisition failed for ${id} (${specification.url}): ${error.message}.${instruction} Existing bundled resources and manifest have not been replaced.`);
  }
}

// Fatal UTF-8 decoding prevents silently replacing malformed corpus bytes.
async function* utf8Lines(stream) {
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let pending = '';
  for await (const bytes of stream) {
    pending += decoder.decode(bytes, { stream: true });
    let start = 0;
    for (;;) {
      const end = pending.indexOf('\n', start);
      if (end < 0) break;
      let line = pending.slice(start, end);
      if (line.endsWith('\r')) line = line.slice(0, -1);
      yield line;
      start = end + 1;
    }
    pending = pending.slice(start);
  }
  pending += decoder.decode();
  if (pending.length) yield pending.endsWith('\r') ? pending.slice(0, -1) : pending;
}

async function prepareWiktextract(path) {
  const languages = { zh: new Map(), ja: new Map() };
  const counts = { totalRecords: 0, selectedRecords: { zh: 0, ja: 0 }, missingGlossRecords: { zh: 0, ja: 0 }, relations: { zh: 0, ja: 0 }, formRelations: { zh: 0, ja: 0 }, resolvedAliases: { zh: 0, ja: 0 }, unresolvedAliases: { zh: 0, ja: 0 } };
  const edges = { zh: new Map(), ja: new Map() };
  const unresolvedSamples = { zh: [], ja: [] };
  const input = createReadStream(path);
  const gunzip = createGunzip();
  input.on('error', (error) => gunzip.destroy(error));
  input.pipe(gunzip);
  try {
    for await (const line of utf8Lines(gunzip)) {
      if (!line.trim()) continue;
      counts.totalRecords++;
      let raw;
      try { raw = JSON.parse(line); } catch (error) { throw new Error(`Wiktextract JSON line ${counts.totalRecords}: ${error.message}`); }
      const language = raw.lang_code === 'ja' ? 'ja' : ['zh', 'cmn'].includes(raw.lang_code) ? 'zh' : null;
      if (!language) continue;
      if (typeof raw.word !== 'string' || !raw.word.trim()) throw new Error(`Selected Wiktextract record ${counts.totalRecords} has no headword`);
      counts.selectedRecords[language]++;
      let entry = languages[language].get(raw.word);
      if (!entry) {
        entry = { headword: raw.word, meanings: [], reading: null, pos: null, aliases: [], sourceUrls: [], forms: [], positions: [] };
        languages[language].set(raw.word, entry);
      }
      if (typeof raw.pos === 'string' && raw.pos.trim()) entry.positions.push(raw.pos);
      entry.sourceUrls.push(pageUrl(raw.word));
      const glosses = [];
      const targets = [];
      for (const sense of Array.isArray(raw.senses) ? raw.senses : []) {
        glosses.push(...strings(sense.glosses));
        for (const relation of ['alt_of', 'form_of']) {
          for (const target of Array.isArray(sense[relation]) ? sense[relation] : []) {
            if (typeof target.word === 'string' && target.word.trim() && target.word !== raw.word) targets.push(target.word);
          }
        }
      }
      if (!glosses.length) counts.missingGlossRecords[language]++;
      entry.meanings.push(...glosses);
      for (const form of Array.isArray(raw.forms) ? raw.forms : []) {
        if (typeof form.form === 'string' && form.form.trim() && form.form !== raw.word) entry.forms.push(form.form);
      }
      // Redirects and kana are not guessed. Only extractor-authored forms and
      // sense alt_of/form_of relations create alias edges.
      if (targets.length) {
        const previous = edges[language].get(raw.word) || [];
        edges[language].set(raw.word, orderedUnique([...previous, ...targets]));
      }
    }
  } finally { input.destroy(); gunzip.destroy(); }

  for (const language of ['zh', 'ja']) {
    const entries = languages[language];
    counts.relations[language] = [...edges[language].values()].reduce((count, targets) => count + targets.length, 0);
    for (const entry of entries.values()) {
      entry.meanings = orderedUnique(entry.meanings);
      entry.pos = orderedUnique(entry.positions).join('; ') || null;
    }
    // Reverse genuine form declarations into the same alias graph. A sense
    // can point at a form that has no separate page/headword; resolving only
    // sense relations would incorrectly omit these usable aliases.
    for (const entry of entries.values()) {
      for (const form of orderedUnique(entry.forms)) {
        const previous = edges[language].get(form) || [];
        edges[language].set(form, orderedUnique([...previous, entry.headword]));
        counts.formRelations[language]++;
      }
    }
    // Iterative traversal resolves chains/cycles without recursion overflow.
    const resolveTargets = (word) => {
      const found = new Set();
      const seen = new Set([word]);
      const queue = [...(edges[language].get(word) || [])];
      for (let i = 0; i < queue.length; i++) {
        const target = queue[i];
        if (seen.has(target)) continue;
        seen.add(target);
        if (entries.get(target)?.meanings.length) found.add(target);
        else queue.push(...(edges[language].get(target) || []));
      }
      return [...found].sort(compare);
    };
    for (const [word, targets] of edges[language]) {
      const resolved = resolveTargets(word);
      if (!resolved.length) {
        counts.unresolvedAliases[language]++;
        if (unresolvedSamples[language].length < 25) unresolvedSamples[language].push({ alias: word, targets });
      } else {
        counts.resolvedAliases[language]++;
        for (const headword of resolved) {
          const canonical = entries.get(headword);
          canonical.aliases.push(word);
          // A form may belong to several unrelated headwords. Each already
          // retains its declaring page; add real alias pages only, never every
          // page that happens to declare the same ambiguous surface form.
          if (entries.has(word)) canonical.sourceUrls.push(pageUrl(word));
        }
      }
    }
    for (const entry of entries.values()) {
      entry.aliases = orderedUnique(entry.aliases.filter((alias) => alias !== entry.headword)).sort(compare);
      entry.sourceUrls = orderedUnique(entry.sourceUrls).sort(compare);
      delete entry.forms;
      delete entry.positions;
    }
    languages[language] = [...entries.values()].filter((entry) => entry.meanings.length > 0).sort((a, b) => compare(a.headword, b.headword));
    unresolvedSamples[language].sort((a, b) => compare(a.alias, b.alias));
  }
  return { languages, report: { ...counts, unresolvedSamples, glossPolicy: 'Only nonempty original Vietnamese-edition senses[].glosses; no generated, English, or fallback definitions.', aliasPolicy: 'Only forms[].form and senses[].alt_of/form_of; combined form/relation chains resolve to existing gloss-bearing headwords. Unresolved aliases are not emitted.' } };
}

async function readAt(handle, position, length) {
  const buffer = Buffer.alloc(length);
  let offset = 0;
  while (offset < length) {
    const { bytesRead } = await handle.read(buffer, offset, length - offset, position + offset);
    if (!bytesRead) throw new Error('Truncated Unihan ZIP');
    offset += bytesRead;
  }
  return buffer;
}

// Read only the ZIP directory into memory; every Unihan text member is inflated
// as a byte stream. ZIP64/encrypted archives are rejected rather than misread.
async function zipMembers(path) {
  const handle = await open(path, 'r');
  try {
    const { size } = await handle.stat();
    const end = await readAt(handle, Math.max(0, size - 65_557), Math.min(size, 65_557));
    let eocd = -1;
    for (let i = end.length - 22; i >= 0; i--) {
      if (end.readUInt32LE(i) === 0x06054b50 && i + 22 + end.readUInt16LE(i + 20) === end.length) { eocd = i; break; }
    }
    if (eocd < 0) throw new Error('Unihan input is not a complete ZIP archive');
    const count = end.readUInt16LE(eocd + 10);
    const directorySize = end.readUInt32LE(eocd + 12);
    const directoryOffset = end.readUInt32LE(eocd + 16);
    if (end.readUInt16LE(eocd + 4) || end.readUInt16LE(eocd + 6) || count === 0xffff || directorySize === 0xffffffff || directoryOffset === 0xffffffff) throw new Error('Unsupported multi-disk/ZIP64 Unihan archive');
    if (directorySize > 16_777_216 || directoryOffset + directorySize > size) throw new Error('Invalid Unihan ZIP directory');
    const directory = await readAt(handle, directoryOffset, directorySize);
    const members = [];
    let offset = 0;
    for (let i = 0; i < count; i++) {
      if (offset + 46 > directory.length || directory.readUInt32LE(offset) !== 0x02014b50) throw new Error('Malformed Unihan ZIP central directory');
      const flags = directory.readUInt16LE(offset + 8);
      const method = directory.readUInt16LE(offset + 10);
      const compressedSize = directory.readUInt32LE(offset + 20);
      const uncompressedSize = directory.readUInt32LE(offset + 24);
      const nameLength = directory.readUInt16LE(offset + 28);
      const extraLength = directory.readUInt16LE(offset + 30);
      const commentLength = directory.readUInt16LE(offset + 32);
      const localOffset = directory.readUInt32LE(offset + 42);
      const next = offset + 46 + nameLength + extraLength + commentLength;
      if (next > directory.length) throw new Error('Truncated Unihan ZIP directory record');
      const name = directory.subarray(offset + 46, offset + 46 + nameLength).toString('utf8');
      if (/^(?:.*\/)?Unihan[^/]*\.txt$/.test(name)) {
        if ((flags & 1) || ![0, 8].includes(method) || compressedSize === 0xffffffff || uncompressedSize === 0xffffffff || localOffset === 0xffffffff) throw new Error(`Unsupported compression/encryption in ${name}`);
        const header = await readAt(handle, localOffset, 30);
        if (header.readUInt32LE(0) !== 0x04034b50) throw new Error(`Invalid local ZIP header: ${name}`);
        const start = localOffset + 30 + header.readUInt16LE(26) + header.readUInt16LE(28);
        if (start + compressedSize > directoryOffset) throw new Error(`Out-of-bounds ZIP member: ${name}`);
        members.push({ name, method, compressedSize, uncompressedSize, start });
      }
      offset = next;
    }
    if (!members.length) throw new Error('Archive contains no Unihan*.txt members');
    return members.sort((a, b) => compare(a.name, b.name));
  } finally { await handle.close(); }
}

async function prepareUnihan(path) {
  const members = await zipMembers(path);
  const entries = new Map();
  const releases = new Set();
  let readingRecords = 0;
  for (const member of members) {
    if (!member.compressedSize) continue;
    const input = createReadStream(path, { start: member.start, end: member.start + member.compressedSize - 1 });
    const stream = member.method === 8 ? createInflateRaw() : input;
    if (stream !== input) { input.on('error', (error) => stream.destroy(error)); input.pipe(stream); }
    let inflated = 0;
    const meter = new Transform({ transform(chunk, _encoding, callback) { inflated += chunk.length; callback(null, chunk); } });
    stream.on('error', (error) => meter.destroy(error));
    stream.pipe(meter);
    try {
      for await (const line of utf8Lines(meter)) {
        const version = line.match(/^#\s*(?:Unicode\s+)?Version\s*:?\s*(\d+\.\d+(?:\.\d+)?)/i);
        if (version) releases.add(version[1]);
        if (!line || line.startsWith('#')) continue;
        const columns = line.split('\t');
        if (columns[1] !== 'kVietnamese') continue;
        if (columns.length !== 3 || !/^U\+[0-9A-F]{4,6}$/.test(columns[0])) throw new Error(`Malformed kVietnamese line in ${member.name}`);
        const scalar = Number.parseInt(columns[0].slice(2), 16);
        if (scalar > 0x10ffff || (scalar >= 0xd800 && scalar <= 0xdfff)) throw new Error('Unihan contains an invalid Unicode scalar');
        const word = String.fromCodePoint(scalar);
        // Only derived Vietnamese readings are normalized/case-adapted.
        const readings = columns[2].trim().split(/\s+/u).filter(Boolean).map((reading) => reading.normalize('NFC').toLocaleLowerCase('vi'));
        if (!readings.length) throw new Error(`Empty kVietnamese reading for ${columns[0]}`);
        const previous = entries.get(word) || { headword: word, meanings: [], reading: null, pos: null, aliases: [], sourceUrls: ['https://www.unicode.org/reports/tr38/', `https://www.unicode.org/cgi-bin/GetUnihanData.pl?codepoint=${columns[0].slice(2)}`] };
        previous.meanings = orderedUnique([...previous.meanings, ...readings]);
        previous.reading = previous.meanings[0];
        entries.set(word, previous);
        readingRecords++;
      }
      if (inflated !== member.uncompressedSize) throw new Error(`Uncompressed size mismatch for ${member.name}`);
    } finally { input.destroy(); if (stream !== input) stream.destroy(); meter.destroy(); }
  }
  if (!entries.size) throw new Error('Unihan archive contains no kVietnamese records');
  if (releases.size !== 1) throw new Error(`Could not identify one Unicode release in archive headers: ${[...releases].join(', ') || 'none'}`);
  return { entries: [...entries.values()].sort((a, b) => compare(a.headword, b.headword)), report: { release: [...releases][0], scannedMembers: members.map((member) => member.name), readingRecords } };
}

async function writeJsonl(path, entries) {
  const hash = createHash('sha256');
  let bytes = 0;
  await pipeline(Readable.from((function* () { for (const entry of entries) yield `${JSON.stringify(entry)}\n`; })()), new Transform({ transform(chunk, _encoding, callback) { hash.update(chunk); bytes += chunk.length; callback(null, chunk); } }), createWriteStream(path, { flags: 'wx' }));
  return { sha256: hash.digest('hex'), bytes, entryCount: entries.length, meaningCount: entries.reduce((n, entry) => n + entry.meanings.length, 0), aliasCount: entries.reduce((n, entry) => n + entry.aliases.length, 0) };
}

function validEntry(entry, file, line) {
  const required = ['headword', 'meanings', 'reading', 'pos', 'aliases', 'sourceUrls'];
  if (!entry || typeof entry !== 'object' || Object.keys(entry).sort().join(',') !== required.sort().join(',') || typeof entry.headword !== 'string' || !entry.headword.trim() || !Array.isArray(entry.meanings) || !entry.meanings.length || entry.meanings.some((meaning) => typeof meaning !== 'string' || !meaning.trim()) || !['reading', 'pos'].every((key) => entry[key] === null || typeof entry[key] === 'string') || !['aliases', 'sourceUrls'].every((key) => Array.isArray(entry[key]) && entry[key].every((value) => typeof value === 'string' && value.trim()))) throw new Error(`Invalid resource entry ${file}:${line}`);
  for (const key of ['meanings', 'aliases', 'sourceUrls']) if (new Set(entry[key]).size !== entry[key].length) throw new Error(`Duplicate ${key} at ${file}:${line}`);
  if (!entry.sourceUrls.length) throw new Error(`Missing provenance at ${file}:${line}`);
  if (entry.aliases.includes(entry.headword)) throw new Error(`Self alias at ${file}:${line}`);
  if (file === 'han-viet.jsonl' && ([...entry.headword].length !== 1 || entry.reading !== entry.meanings[0])) throw new Error(`Invalid Hán Việt entry at ${file}:${line}`);
}

async function verify(manifest) {
  if (!manifest) throw new Error('No bundled dictionary manifest. Run npm run data:prepare (online acquisition), or supply --wiktextract PATH --unihan PATH. A build may not use placeholder dictionaries.');
  for (const file of DATA_FILES) {
    const descriptor = manifest.resources.find((resource) => resource.file === file);
    if (!descriptor) throw new Error(`Manifest missing ${file}`);
    const path = join(OUT, file);
    if (!await exists(path)) throw new Error(`Missing bundled resource ${file}. Run npm run data:prepare to acquire/regenerate the checksum-locked snapshot.`);
    requireDigest(await digest(path), descriptor, file);
    let entryCount = 0;
    let meaningCount = 0;
    let aliasCount = 0;
    let previous = null;
    for await (const line of utf8Lines(createReadStream(path))) {
      entryCount++;
      let entry;
      try { entry = JSON.parse(line); } catch (error) { throw new Error(`Invalid JSON ${file}:${entryCount}: ${error.message}`); }
      validEntry(entry, file, entryCount);
      if (previous !== null && compare(previous, entry.headword) >= 0) throw new Error(`Unsorted/duplicate headword at ${file}:${entryCount}`);
      previous = entry.headword;
      meaningCount += entry.meanings.length;
      aliasCount += entry.aliases.length;
    }
    if (!entryCount || entryCount !== descriptor.entryCount || meaningCount !== descriptor.meaningCount || aliasCount !== descriptor.aliasCount) throw new Error(`Resource counts differ from manifest for ${file}`);
  }
  for (const file of NOTICE_FILES) {
    const descriptor = manifest.notices.find((record) => record.file === file);
    if (!descriptor) throw new Error(`Manifest missing notice ${file}`);
    const path = join(OUT, file);
    if (!await exists(path)) throw new Error(`Missing bundled notice ${file}. Run npm run data:prepare to regenerate the checksum-locked offline bundle.`);
    requireDigest(await digest(path), descriptor, file);
  }
  return manifest;
}

function printReport(manifest, action) {
  console.log(JSON.stringify({ action, format: manifest.format, version: manifest.version, snapshots: manifest.snapshots, resources: manifest.resources, preparationReport: manifest.preparationReport }, null, 2));
}

async function main() {
  const options = args();
  let previous;
  try { previous = await loadManifest(); } catch (error) { if (!options.refresh) throw error; console.error(`${error.message} Explicit refresh will replace this manifest only after successful preparation.`); }
  for (const id of ['wiktextract', 'unihan']) {
    if (options[id] && previous && !options.refresh) requireDigest(await digest(options[id]), previous.sources[id], `Local ${id}`);
  }
  if (options.verify) { printReport(await verify(previous), 'verified-offline'); return; }
  if (!options.refresh && previous && (await Promise.all([...DATA_FILES, ...NOTICE_FILES].map((file) => exists(join(OUT, file))))).every(Boolean)) {
    printReport(await verify(previous), 'reused-offline');
    return;
  }
  if (!options.refresh && previous && previous.scriptVersion !== SCRIPT_VERSION) throw new Error('Preparation transformation version changed. Explicit --refresh is required to update locked resources.');
  const inputs = {};
  for (const id of ['wiktextract', 'unihan']) inputs[id] = await acquire(id, SOURCES[id], options, previous?.sources[id], options[id]);
  const sameLockedSnapshot = previous?.sources.wiktextract?.sha256 === inputs.wiktextract.metadata.sha256;
  let dumpDate = sameLockedSnapshot ? previous.snapshots.wiktionaryDump : null;
  let extractionDate = sameLockedSnapshot ? previous.snapshots.wiktextractExtraction : null;
  if (options.wiktextract || inputs.wiktextract.metadata.suppliedLocally) {
    // A current web page cannot date an arbitrary user-supplied historical
    // archive. Exact locked copies inherit known dates; otherwise retrieval
    // time and digest are recorded and unknown snapshot dates stay null.
    if (sameLockedSnapshot && previous.sources.wiktextractMetadata) inputs.wiktextractMetadata = { path: null, metadata: previous.sources.wiktextractMetadata };
    else console.error('Local Wiktextract snapshot dates are unknown; manifest records the observed digest and retrieval date, not dates from an unrelated current web page.');
  } else {
    try {
      inputs.wiktextractMetadata = await acquire('wiktextractMetadata', SOURCES.wiktextractMetadata, options, previous?.sources.wiktextractMetadata);
    } catch (error) {
      if (!sameLockedSnapshot || !dumpDate || !extractionDate) throw error;
      inputs.wiktextractMetadata = { path: null, metadata: previous.sources.wiktextractMetadata };
      console.error('Source metadata unavailable; retaining dates from the identical checksum-locked snapshot.');
    }
    if (inputs.wiktextractMetadata.path) {
      const metadata = await readFile(inputs.wiktextractMetadata.path, 'utf8');
      dumpDate = metadata.match(/dump(?:<[^>]+>|\s)*dated\s*(\d{4}-\d{2}-\d{2})/i)?.[1];
      extractionDate = metadata.match(/extracted on\s*(\d{4}-\d{2}-\d{2})/i)?.[1];
      if (!dumpDate || !extractionDate) throw new Error('Cannot identify Vietnamese Wiktionary dump/extraction snapshot dates in source metadata. No bundle was published.');
    }
  }
  const notices = {};
  for (const notice of NOTICES) {
    const id = `license-${notice.id}`;
    const bundled = join(OUT, notice.id === 'ipadic' ? IPADIC_ORIGINAL : notice.file);
    if (!options.refresh && previous?.sources[id] && await exists(bundled)) {
      requireDigest(await digest(bundled), previous.sources[id], notice.file);
      notices[notice.id] = { path: bundled, metadata: previous.sources[id] };
    } else notices[notice.id] = await acquire(id, { url: notice.url, extension: '.txt' }, options, previous?.sources[id]);
  }
  console.error('Streaming Vietnamese Wiktextract records and all Unihan text members…');
  const wiktextract = await prepareWiktextract(inputs.wiktextract.path);
  const unihan = await prepareUnihan(inputs.unihan.path);
  if (!wiktextract.languages.zh.length || !wiktextract.languages.ja.length) throw new Error('Vietnamese raw input contains no usable Chinese or Japanese subset; no bundle was published.');
  await mkdir(OUT, { recursive: true });
  const stage = await mkdtemp(join(OUT, '.prepare-'));
  try {
    await mkdir(join(stage, 'licenses'));
    const resources = [];
    for (const [file, entries, source, license] of [
      ['zh-vi.jsonl', wiktextract.languages.zh, 'wiktextract', 'CC-BY-SA-4.0'],
      ['ja-vi.jsonl', wiktextract.languages.ja, 'wiktextract', 'CC-BY-SA-4.0'],
      ['han-viet.jsonl', unihan.entries, 'unihan', 'Unicode-3.0'],
    ]) resources.push({ file, ...await writeJsonl(join(stage, file), entries), source, license });
    const noticeRecords = [];
    for (const notice of NOTICES) {
      const original = await readFile(notices[notice.id].path);
      let text;
      try { text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(original); }
      catch (error) {
        if (notice.id !== 'ipadic') throw new Error(`License ${notice.file} is not valid UTF-8: ${error.message}`);
        // Upstream's English notice ends with invalid bytes F7 F7. Do not
        // guess an encoding, replace characters silently, or remove terms.
        text = 'Display copy: non-UTF-8 bytes are escaped as \\xHH below.\nByte-exact original: IPADIC-original.COPYING\n\n';
        for (const byte of original) text += byte < 0x80 ? String.fromCharCode(byte) : `\\x${byte.toString(16).toUpperCase().padStart(2, '0')}`;
      }
      await writeFile(join(stage, notice.file), text, 'utf8');
      noticeRecords.push({ file: notice.file, ...await digest(join(stage, notice.file)), license: notice.license, sourceUrl: notice.url, encoding: 'UTF-8' });
      if (notice.id === 'ipadic') {
        await writeFile(join(stage, IPADIC_ORIGINAL), original);
        noticeRecords.push({ file: IPADIC_ORIGINAL, ...await digest(join(stage, IPADIC_ORIGINAL)), license: notice.license, sourceUrl: notice.url, encoding: 'original bytes (upstream includes non-UTF-8 bytes)' });
      }
    }
    const attribution = `QuickTranslator offline resources — attribution and redistribution

Chinese/Vietnamese and Japanese/Vietnamese derived datasets
Files: zh-vi.jsonl, ja-vi.jsonl
Copyright: Vietnamese Wiktionary contributors. Individual source page links
(including contributor history) are retained in every entry's sourceUrls.
Source edition: https://vi.wiktionary.org/
Raw structured extraction: https://kaikki.org/viwiktionary/rawdata.html
Wikimedia dump snapshot: ${dumpDate ?? 'unknown (locally supplied archive)'}.
Wiktextract extraction: ${extractionDate ?? 'unknown (locally supplied archive)'}.
Observed archive retrieval date and digest are recorded in manifest.json.
Extractor: https://github.com/tatuylonen/wiktextract
License for these derived datasets: Creative Commons Attribution-ShareAlike 4.0
International, https://creativecommons.org/licenses/by-sa/4.0/
Full terms: licenses/CC-BY-SA-4.0.txt. These datasets are distributed separately
from application code. Redistribution/adaptation must retain attribution,
indicate changes, and use the same or a compatible share-alike license.
No endorsement by Wikimedia, Wiktionary, Kaikki or the contributors is implied.
Modifications: selected all ja and zh/cmn entries with Vietnamese glosses; merged
ordered unique glosses/POS and genuine forms/alt_of/form_of aliases; retained
source page attribution; omitted entries without glosses and unresolved aliases;
sorted by headword and serialized the documented six-field JSONL schema.
Definitions are dictionary lookup assistance, not fluent machine translation.

Hán Việt readings
File: han-viet.jsonl; source: Unicode Unihan, release ${unihan.report.release}.
https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip
https://www.unicode.org/reports/tr38/
Copyright: Unicode, Inc. License: Unicode License V3.
Full copyright and permission notice: licenses/Unicode-V3.txt.
Modifications: extracted every kVietnamese record from all Unihan*.txt files,
NFC-normalized/lowercased readings only, preserved ordered alternatives,
selected the first reading, and sorted records.
Source characters are never normalized, simplified or case-folded.

Japanese tokenizer
Lindera: https://github.com/lindera/lindera, MIT; licenses/Lindera-MIT.txt.
IPADIC: https://github.com/lindera/mecab-ipadic, NAIST/ICOT conditions;
licenses/IPADIC.txt. The embedded IPADIC dictionary ships through Lindera, not
through these gloss files. Notices accompany the application's offline bundle.
Notices retain their original terms; IPADIC.txt visibly escapes non-UTF-8 bytes
in the upstream notice instead of guessing an encoding or replacing them.
licenses/IPADIC-original.COPYING retains its byte-exact original. Original
download digests remain recorded in manifest.json.

All resources are provided without warranty under their respective licenses.
Acquisition URLs, observed SHA-256 digests, byte/entry counts, transformations
and snapshot metadata are in manifest.json. Raw downloaded inputs are not
redistributed in the application. User-imported/private dictionaries are not
part of these datasets or notices.
`;
    await writeFile(join(stage, 'ATTRIBUTION.txt'), attribution, 'utf8');
    noticeRecords.push({ file: 'ATTRIBUTION.txt', ...await digest(join(stage, 'ATTRIBUTION.txt')), license: 'resource-specific', sourceUrl: 'https://kaikki.org/viwiktionary/rawdata.html' });
    const sources = Object.fromEntries(Object.entries(inputs).map(([id, input]) => [id, input.metadata]));
    for (const notice of NOTICES) sources[`license-${notice.id}`] = notices[notice.id].metadata;
    const manifest = {
      format: 'quicktranslator-dictionaries', version: 1, scriptVersion: SCRIPT_VERSION,
      generatedAt: new Date().toISOString(),
      entrySchema: { headword: 'string', meanings: 'string[] (ordered)', reading: 'string|null', pos: 'string|null', aliases: 'string[]', sourceUrls: 'string[]' },
      ordering: 'JavaScript UTF-16 code-unit lexical order; meanings in original input order; aliases/sourceUrls sorted and unique',
      sources,
      snapshots: { wiktionaryDump: dumpDate, wiktextractExtraction: extractionDate, unicodeRelease: unihan.report.release },
      resources, notices: noticeRecords,
      transformations: [
        'Stream and strictly decode gzip JSONL; filter lang_code ja, zh and cmn from the Vietnamese Wiktionary edition.',
        'Merge all nonempty senses[].glosses by headword in original record/sense order; retain original text and stable unique meanings.',
        'Merge observed pos labels in input order into a semicolon-separated string; leave unavailable readings null.',
        'Index extractor-authored forms[].form and senses[].alt_of/form_of; resolve alias chains/cycles to gloss-bearing headwords; never invent glosses.',
        'Retain Vietnamese Wiktionary source-page URLs for headwords and relation aliases, including contributors via page history.',
        'Stream every Unihan*.txt ZIP member; extract all kVietnamese alternatives; normalize NFC/lowercase only derived Vietnamese readings.',
        'Keep source characters unchanged; sort headwords/aliases/provenance deterministically; emit one six-field UTF-8 JSON object per line.',
        'Bundle byte-exact IPADIC COPYING and a UTF-8 display copy with explicitly escaped non-UTF-8 bytes; never discard license text.',
      ],
      preparationReport: { wiktextract: wiktextract.report, unihan: unihan.report },
    };
    if (previous && !options.refresh) {
      for (const resource of resources) requireDigest(resource, previous.resources.find((entry) => entry.file === resource.file), `Regenerated ${resource.file}`);
      for (const notice of noticeRecords) requireDigest(notice, previous.notices.find((entry) => entry.file === notice.file), `Regenerated ${notice.file}`);
      // Reusing a locked snapshot must not create timestamp-only manifest churn.
      manifest.generatedAt = previous.generatedAt;
    }
    await writeFile(join(stage, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
    await mkdir(join(OUT, 'licenses'), { recursive: true });
    for (const file of [...DATA_FILES, ...NOTICE_FILES]) await rename(join(stage, file), join(OUT, file));
    // Publish manifest last. Interrupted publication cannot pass --verify/build
    // integrity checks with mismatched old and new resources.
    await rename(join(stage, 'manifest.json'), MANIFEST);
    printReport(manifest, options.refresh ? 'refreshed' : 'prepared');
  } finally { await rm(stage, { recursive: true, force: true }); }
}

main().catch((error) => { console.error(`Dictionary preparation failed: ${error.message}`); process.exitCode = 1; });
