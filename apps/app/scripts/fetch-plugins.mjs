// Replaces the bundled plugin baselines in public/plugins with the latest signed GitHub releases.
// Mirrors the launcher's checks in packages/core/src/plugins.rs. Never fails the build: on any problem
// the existing bundled copy is kept and the reason is printed.
// FETCH_PLUGINS_BASE overrides https://github.com (used to test the offline path).
import { createHash, createPublicKey, verify } from "node:crypto";
import { mkdir, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const PLUGINS = [
  { id: "ratscan", repo: "hhhhhffd/ratka_checker" },
  { id: "packconv", repo: "hhhhhffd/conventor-mc-rp" },
];
const API = 1;
const PUBLIC_KEY = "b37Axim0D7VgDNWVIgvlnpdn2nVOMSLM/jIFbRFc/4k=";
const MESSAGE_PREFIX = Buffer.from("_k0t1k-plugin\n");
const MAX_MANIFEST_BYTES = 64 * 1024;
const MAX_SIGNATURE_BYTES = 1024;
const MAX_FILE_BYTES = 32 * 1024 * 1024;
const MAX_FILES = 8;
const BASE = process.env.FETCH_PLUGINS_BASE ?? "https://github.com";

const here = dirname(fileURLToPath(import.meta.url));
const pluginsDir = join(here, "..", "public", "plugins");

const key = createPublicKey({
  key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(PUBLIC_KEY, "base64")]),
  format: "der",
  type: "spki",
});

const parseVersion = (text) => {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(String(text).trim());
  return match ? match.slice(1).map(Number) : null;
};
const compare = (a, b) => a.reduce((diff, part, i) => diff || part - b[i], 0);
const validFileName = (name) => /^[A-Za-z0-9_-][A-Za-z0-9._-]*$/.test(name);

const launcherVersion = (await readFile(join(here, "..", "..", "..", "Cargo.toml"), "utf8"))
  .match(/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/)?.[1];
if (!parseVersion(launcherVersion)) {
  console.warn(`fetch-plugins: cannot read the workspace version (${launcherVersion}), keeping all bundled plugins`);
  process.exit(0);
}

async function download(plugin, asset, limit) {
  const url = `${BASE}/${plugin.repo}/releases/latest/download/${asset}`;
  const response = await fetch(url, { signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw new Error(`${asset}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > limit) throw new Error(`${asset} is larger than ${limit} bytes`);
  return bytes;
}

async function fetchRelease(plugin) {
  const manifestBytes = await download(plugin, "plugin.json", MAX_MANIFEST_BYTES);
  const signature = Buffer.from((await download(plugin, "plugin.json.sig", MAX_SIGNATURE_BYTES)).toString().trim(), "base64");
  if (!verify(null, Buffer.concat([MESSAGE_PREFIX, manifestBytes]), key, signature)) throw new Error("bad signature");

  const manifest = JSON.parse(manifestBytes.toString());
  if (manifest.id !== plugin.id) throw new Error(`manifest is for ${JSON.stringify(manifest.id)}`);
  if (manifest.api !== API) throw new Error(`needs plugin api ${manifest.api}, expected ${API}`);
  const wanted = parseVersion(manifest.minLauncher);
  if (!wanted) throw new Error(`bad minLauncher ${JSON.stringify(manifest.minLauncher)}`);
  if (compare(wanted, parseVersion(launcherVersion)) > 0) throw new Error(`needs launcher ${manifest.minLauncher}, this is ${launcherVersion}`);
  if (!parseVersion(manifest.version)) throw new Error(`bad version ${JSON.stringify(manifest.version)}`);
  const entries = Object.entries(manifest.files ?? {});
  if (entries.length === 0 || entries.length > MAX_FILES) throw new Error("unexpected number of files");
  for (const [name, hash] of entries) {
    if (!validFileName(name) || name === "plugin.json" || !/^[0-9a-f]{64}$/.test(hash)) throw new Error(`bad file entry ${JSON.stringify(name)}`);
  }

  const files = new Map([["plugin.json", manifestBytes]]);
  for (const [name, hash] of entries) {
    const bytes = await download(plugin, name, MAX_FILE_BYTES);
    const actual = createHash("sha256").update(bytes).digest("hex");
    if (actual !== hash) throw new Error(`${name} has sha256 ${actual}, plugin.json says ${hash}`);
    files.set(name, bytes);
  }
  return { version: manifest.version, files };
}

// Stage next to the target, then swap, so a failure never leaves a half-replaced folder.
async function replaceFolder(dir, files) {
  const staging = `${dir}.new`;
  const backup = `${dir}.old`;
  await rm(staging, { recursive: true, force: true });
  await rm(backup, { recursive: true, force: true });
  await mkdir(staging, { recursive: true });
  try {
    for (const [name, bytes] of files) await writeFile(join(staging, name), bytes);
    await rename(dir, backup);
    await rename(staging, dir);
  } catch (error) {
    await rm(staging, { recursive: true, force: true });
    await rm(dir, { recursive: true, force: true }).catch(() => {});
    await rename(backup, dir).catch(() => {});
    throw error;
  }
  await rm(backup, { recursive: true, force: true });
}

for (const plugin of PLUGINS) {
  const dir = join(pluginsDir, plugin.id);
  let current = "unknown";
  try {
    current = JSON.parse(await readFile(join(dir, "plugin.json"), "utf8")).version;
    const release = await fetchRelease(plugin);
    if (release.version === current) {
      console.log(`fetch-plugins: ${plugin.id} ${current} is up to date`);
    } else if (compare(parseVersion(release.version), parseVersion(current)) < 0) {
      console.warn(`fetch-plugins: ${plugin.id} kept ${current}: latest release ${release.version} is older`);
    } else {
      await replaceFolder(dir, release.files);
      console.log(`fetch-plugins: ${plugin.id} updated ${current} -> ${release.version}`);
    }
  } catch (error) {
    console.warn(`fetch-plugins: WARNING ${plugin.id} kept ${current} because ${error.cause?.message ?? error.message}`);
  }
}
