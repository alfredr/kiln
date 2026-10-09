import wasmInit, {
  compile_typst_init,
  add_typst_package_file,
  compile_typst,
} from '../explorer/pkg/kiln_doc_wasm.js';

// --- Package cache: "preview/name/version" → Map<filepath, Uint8Array> ---
const pkgCache = new Map();

// --- Tar + gzip helpers ---

async function decompressGzip(buffer) {
  const ds = new DecompressionStream('gzip');
  const reader = new Blob([buffer]).stream().pipeThrough(ds).getReader();
  const chunks = [];
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
  }
  let total = 0;
  for (const c of chunks) total += c.length;
  const out = new Uint8Array(total);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.length;
  }
  return out;
}

function parseTar(data) {
  const files = new Map();
  const decoder = new TextDecoder();
  let offset = 0;
  while (offset + 512 <= data.length) {
    const header = data.subarray(offset, offset + 512);
    // Empty block signals end of archive
    if (header.every(b => b === 0)) break;

    const nameRaw = decoder.decode(header.subarray(0, 100)).replace(/\0.*/, '');
    const sizeOctal = decoder.decode(header.subarray(124, 136)).replace(/\0.*/, '').trim();
    const size = parseInt(sizeOctal, 8) || 0;
    const typeFlag = header[156];

    offset += 512; // advance past header

    if (size > 0 && (typeFlag === 48 || typeFlag === 0)) {
      // typeFlag 48 = '0' (regular file), 0 = legacy regular
      if (nameRaw && nameRaw !== '.') {
        files.set(nameRaw, data.slice(offset, offset + size));
      }
    }

    // Data blocks are padded to 512-byte boundaries
    offset += Math.ceil(size / 512) * 512;
  }
  return files;
}

// --- Package fetching ---

function parsePackageSpecs(text) {
  const specs = new Map();
  const re = /@(\w+)\/([\w-]+):([\d.]+)/g;
  let m;
  while ((m = re.exec(text)) !== null) {
    const [, namespace, name, version] = m;
    const key = `${namespace}/${name}/${version}`;
    if (!specs.has(key)) {
      specs.set(key, { namespace, name, version });
    }
  }
  return specs;
}

async function fetchPackage(spec, setStatus) {
  const key = `${spec.namespace}/${spec.name}/${spec.version}`;
  if (pkgCache.has(key)) return pkgCache.get(key);

  setStatus(`Fetching ${spec.name} ${spec.version}...`);

  const url = `https://packages.typst.org/${spec.namespace}/${spec.name}-${spec.version}.tar.gz`;
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`Failed to fetch package ${key}: ${resp.status}`);

  const compressed = await resp.arrayBuffer();
  const tarData = await decompressGzip(compressed);
  const files = parseTar(tarData);

  pkgCache.set(key, files);
  return files;
}

async function fetchAllPackages(text, setStatus) {
  // Breadth-first: parse imports, fetch, scan for transitive deps, repeat
  const fetched = new Set();
  let toScan = text;

  while (true) {
    const specs = parsePackageSpecs(toScan);
    const newSpecs = [];
    for (const [key, spec] of specs) {
      if (!fetched.has(key)) {
        newSpecs.push([key, spec]);
      }
    }
    if (newSpecs.length === 0) break;

    let newText = '';
    for (const [key, spec] of newSpecs) {
      const files = await fetchPackage(spec, setStatus);
      fetched.add(key);
      // Scan .typ files for transitive imports
      const decoder = new TextDecoder();
      for (const [path, data] of files) {
        if (path.endsWith('.typ')) {
          newText += decoder.decode(data) + '\n';
        }
      }
    }
    toScan = newText;
  }
}

function registerAllPackages() {
  for (const [key, files] of pkgCache) {
    for (const [path, data] of files) {
      add_typst_package_file(`${key}/${path}`, new Uint8Array(data));
    }
  }
}

// --- Main ---

const btn = document.getElementById('compile-btn');
const statusEl = document.getElementById('status');
const preambleEl = document.getElementById('preamble');
const sourceEl = document.getElementById('source');
const outputEl = document.getElementById('output');

function setStatus(msg, kind) {
  statusEl.textContent = msg;
  statusEl.className = kind || '';
}

async function compile() {
  btn.disabled = true;
  setStatus('Preparing...', '');

  try {
    const preamble = preambleEl.value;
    const source = sourceEl.value;

    // Fetch any referenced packages
    await fetchAllPackages(preamble + '\n' + source, (msg) => setStatus(msg, ''));

    setStatus('Compiling...', '');

    // Init world
    const initResult = compile_typst_init(source, JSON.stringify({ preamble }));
    if (initResult !== 'ok') {
      const err = JSON.parse(initResult);
      throw new Error(err.error);
    }

    // Register cached packages
    registerAllPackages();

    // Compile
    const result = compile_typst();

    if (result.startsWith('{')) {
      const err = JSON.parse(result);
      if (err.error) throw new Error(err.error);
    }

    outputEl.innerHTML = result;
    setStatus('Done', 'ok');
  } catch (e) {
    outputEl.innerHTML = `<div class="error-msg">${escapeHtml(e.message)}</div>`;
    setStatus('Error', 'error');
  } finally {
    btn.disabled = false;
  }
}

function escapeHtml(s) {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

// Ctrl/Cmd+Enter to compile
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter' && !btn.disabled) {
    e.preventDefault();
    compile();
  }
});

btn.addEventListener('click', compile);

// Init WASM
setStatus('Loading WASM...', '');
wasmInit().then(() => {
  btn.disabled = false;
  btn.textContent = 'Compile';
  setStatus('Ready (Ctrl+Enter)', '');
});
