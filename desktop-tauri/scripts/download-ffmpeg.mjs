#!/usr/bin/env node
/**
 * Downloads pinned ffmpeg static builds (eugeneware/ffmpeg-static b4.4.1)
 * into src-tauri/binaries/ using Tauri sidecar naming:
 *   ffmpeg-x86_64-unknown-linux-gnu
 *   ffmpeg-x86_64-pc-windows-msvc.exe
 * amd64-only per project scope.
 */
import { createHash } from "node:crypto";
import { createReadStream, createWriteStream, existsSync } from "node:fs";
import { chmod, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { createGunzip } from "node:zlib";

const VERSION = "b4.4.1";
const RELEASE_BASE = `https://github.com/eugeneware/ffmpeg-static/releases/download/${VERSION}`;

const BUILDS = {
  "linux-x64": {
    asset: "linux-x64.gz",
    sha256: "e8eb59e6d519eef28e571111dbc200a2c58a4d17be5f3f7eac41fb066d3f1a0f",
    sidecar: "ffmpeg-x86_64-unknown-linux-gnu",
  },
  "win32-x64": {
    asset: "win32-x64.gz",
    sha256: "7277536c2a8cb46c7ee687e8247f906a1c8939b920ea34000e3766501e3a0bf4",
    sidecar: "ffmpeg-x86_64-pc-windows-msvc.exe",
  },
};

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binDir = path.join(root, "src-tauri", "binaries");

async function sha256File(filePath) {
  const hash = createHash("sha256");
  await pipeline(createReadStream(filePath), hash);
  return hash.digest("hex");
}

async function download(url, destPath) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok || !response.body) {
    throw new Error(`ffmpeg download failed: ${response.status} ${response.statusText} (${url})`);
  }
  await pipeline(Readable.fromWeb(response.body), createWriteStream(destPath));
}

async function installOne(key, build, force) {
  const dest = path.join(binDir, build.sidecar);
  const stampPath = `${dest}.version`;
  if (!force && existsSync(dest)) {
    try {
      const stamp = (await readFile(stampPath, "utf8")).trim();
      if (stamp === `${VERSION} ${key}`) {
        console.log(`ffmpeg ${VERSION} (${key}) already present at ${dest}`);
        return;
      }
    } catch {
      // fall through to download
    }
  }
  const archivePath = path.join(tmpdir(), `openmedia-${build.asset}`);
  const tempPath = `${dest}.tmp`;
  const url = `${RELEASE_BASE}/${build.asset}`;
  console.log(`Downloading ffmpeg ${VERSION} (${key})…`);
  await download(url, archivePath);
  const digest = await sha256File(archivePath);
  if (digest !== build.sha256) {
    await rm(archivePath, { force: true });
    throw new Error(
      `ffmpeg checksum mismatch for ${build.asset}\n  expected ${build.sha256}\n  got      ${digest}`
    );
  }
  await pipeline(createReadStream(archivePath), createGunzip(), createWriteStream(tempPath));
  await rm(archivePath, { force: true });
  if (!dest.endsWith(".exe")) {
    await chmod(tempPath, 0o755);
  }
  await rename(tempPath, dest);
  await writeFile(stampPath, `${VERSION} ${key}\n`, "utf8");
  console.log(`Installed ffmpeg ${VERSION} → ${dest}`);
}

async function main() {
  const only = process.env.FFMPEG_ONLY ?? "";
  const force = process.env.FFMPEG_FORCE === "1";
  await mkdir(binDir, { recursive: true });
  for (const [key, build] of Object.entries(BUILDS)) {
    if (only && key !== only) continue;
    await installOne(key, build, force);
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
