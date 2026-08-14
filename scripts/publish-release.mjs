#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  copyFile,
  link,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { arch, platform } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = "iobee/dsh-desktop";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const arguments_ = process.argv.slice(2);
const dryRun = arguments_.includes("--dry-run");
const notesArgument = arguments_.find((argument) => argument !== "--dry-run");
const config = JSON.parse(
  await readFile(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const version = config.version;
const tag = `v${version}`;
const notes =
  notesArgument ??
  "Signed in-app updates, non-blocking background checks, and the latest npm dsh runtime.";

if (platform() !== "darwin" || arch() !== "arm64") {
  throw new Error(`Releases must be built on Apple Silicon macOS, got ${platform()}-${arch()}`);
}

function run(command, args, options = {}) {
  execFileSync(command, args, { cwd: root, stdio: "inherit", ...options });
}

function output(command, args) {
  return execFileSync(command, args, { cwd: root, encoding: "utf8" }).trim();
}

async function copyOrLink(source, target) {
  try {
    await link(source, target);
  } catch {
    await copyFile(source, target);
  }
}

async function sha256(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

run("npm", ["run", "release:verify", "--", tag]);
if (!dryRun) {
  if (output("git", ["branch", "--show-current"]) !== "main") {
    throw new Error("Publishing is only allowed from main");
  }
  if (output("git", ["status", "--porcelain"])) {
    throw new Error("Commit or discard all source changes before publishing");
  }
  run("gh", ["auth", "status"]);
  const existingRelease = spawnSync(
    "gh",
    ["release", "view", tag, "--repo", repository],
    { cwd: root, encoding: "utf8" },
  );
  if (existingRelease.status === 0) {
    throw new Error(`GitHub Release ${tag} already exists`);
  }
  const releaseError = `${existingRelease.stdout ?? ""}\n${existingRelease.stderr ?? ""}`;
  if (!releaseError.includes("release not found")) {
    throw new Error(`Could not verify release availability: ${releaseError.trim()}`);
  }
}

run("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml"]);
run("npm", ["run", "desktop:build"]);

const bundleRoot = join(root, "src-tauri", "target", "release", "bundle");
const dmgDirectory = join(bundleRoot, "dmg");
const dmgCandidates = (await readdir(dmgDirectory)).filter(
  (name) => name.endsWith(".dmg") && name.includes(version),
);
if (dmgCandidates.length !== 1) {
  throw new Error(`Expected one ${version} DMG, found: ${dmgCandidates.join(", ")}`);
}

const sourceDmg = join(dmgDirectory, dmgCandidates[0]);
const sourceUpdate = join(bundleRoot, "macos", `${config.productName}.app.tar.gz`);
const sourceSignature = `${sourceUpdate}.sig`;
run("cargo", [
  "run",
  "--quiet",
  "--manifest-path",
  "src-tauri/Cargo.toml",
  "--example",
  "verify_update_signature",
  "--",
  "src-tauri/tauri.conf.json",
  sourceSignature,
  sourceUpdate,
]);
const publishDirectory = join(root, "src-tauri", "target", "release", "publish");
await rm(publishDirectory, { recursive: true, force: true });
await mkdir(publishDirectory, { recursive: true });

const dmg = join(publishDirectory, `DSH.Desktop_${version}_aarch64.dmg`);
const update = join(publishDirectory, `DSH.Desktop_${version}_aarch64.app.tar.gz`);
const signature = `${update}.sig`;
await copyOrLink(sourceDmg, dmg);
await copyOrLink(sourceUpdate, update);
await copyOrLink(sourceSignature, signature);

const signatureContent = (await readFile(signature, "utf8")).trim();
const publishedAt = new Date().toISOString();
const updateUrl = `https://github.com/${repository}/releases/download/${tag}/${basename(update)}`;
const latest = {
  version,
  notes,
  pub_date: publishedAt,
  platforms: {
    "darwin-aarch64": {
      signature: signatureContent,
      url: updateUrl,
    },
  },
};
const latestPath = join(publishDirectory, "latest.json");
await writeFile(latestPath, `${JSON.stringify(latest, null, 2)}\n`);

const dmgHash = await sha256(dmg);
const updateHash = await sha256(update);
const checksumsPath = join(publishDirectory, "SHA256SUMS.txt");
await writeFile(
  checksumsPath,
  `${dmgHash}  ${basename(dmg)}\n${updateHash}  ${basename(update)}\n`,
);

const runtimeManifest = JSON.parse(
  await readFile(join(root, "src-tauri", "resources", "runtime-manifest.json"), "utf8"),
);
const releaseNotesPath = join(publishDirectory, "release-notes.md");
await writeFile(
  releaseNotesPath,
  `${notes}\n\n` +
    `- Apple Silicon macOS\n` +
    `- Bundled \`@deepseek-ai/dsh@${runtimeManifest.dshVersion}\`\n` +
    `- Ad-hoc signed and not notarized\n\n` +
    `First installation may require:\n\n` +
    "```sh\n" +
    'xattr -dr com.apple.quarantine "/Applications/DSH Desktop.app"\n' +
    "```\n\n" +
    `DMG SHA-256: \`${dmgHash}\`\n`,
);

process.stdout.write(`Prepared signed release files in ${publishDirectory}\n`);
process.stdout.write(`DMG SHA-256: ${dmgHash}\n`);
if (dryRun) {
  process.stdout.write("Dry run complete; nothing was pushed to GitHub.\n");
  process.exit(0);
}

const head = output("git", ["rev-parse", "HEAD"]);
const localTag = spawnSync("git", ["rev-parse", "--verify", `refs/tags/${tag}`], {
  cwd: root,
  encoding: "utf8",
});
if (localTag.status === 0) {
  if (localTag.stdout.trim() !== head) {
    throw new Error(`Local tag ${tag} points to a different commit`);
  }
} else {
  run("git", ["tag", tag]);
}
run("git", ["push", "origin", "main"]);
run("git", ["push", "origin", tag]);
run("gh", [
  "release",
  "create",
  tag,
  `${dmg}#DSH Desktop ${version} (Apple Silicon)`,
  update,
  signature,
  latestPath,
  checksumsPath,
  "--repo",
  repository,
  "--verify-tag",
  "--title",
  `DSH Desktop ${tag}`,
  "--notes-file",
  releaseNotesPath,
  "--latest",
]);
