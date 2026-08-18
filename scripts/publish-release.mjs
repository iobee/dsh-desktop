#!/usr/bin/env node

import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  access,
  copyFile,
  link,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { arch, homedir, platform } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repository = "iobee/dsh-desktop";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const arguments_ = process.argv.slice(2);
let dryRun = false;
let notesArgument;
let windowsInstallerArgument;
for (let index = 0; index < arguments_.length; index += 1) {
  const argument = arguments_[index];
  if (argument === "--dry-run") {
    dryRun = true;
  } else if (argument === "--windows-installer") {
    windowsInstallerArgument = arguments_[index + 1];
    if (!windowsInstallerArgument) throw new Error("--windows-installer requires a path");
    index += 1;
  } else if (argument.startsWith("--")) {
    throw new Error(`Unknown option ${argument}`);
  } else if (notesArgument === undefined) {
    notesArgument = argument;
  } else {
    throw new Error(`Unexpected argument ${argument}`);
  }
}
const windowsInstaller = windowsInstallerArgument
  ? isAbsolute(windowsInstallerArgument)
    ? windowsInstallerArgument
    : resolve(root, windowsInstallerArgument)
  : undefined;
const config = JSON.parse(
  await readFile(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const version = config.version;
const tag = `v${version}`;
const notes =
  notesArgument ??
  "System Node/npm initialization, user-level DSH, and explicit desktop/runtime updates.";

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

async function updaterSigningEnvironment() {
  const environment = { ...process.env };
  if (!environment.TAURI_SIGNING_PRIVATE_KEY && !environment.TAURI_SIGNING_PRIVATE_KEY_PATH) {
    const defaultKey = join(homedir(), ".tauri", "dsh-desktop.key");
    await access(defaultKey);
    environment.TAURI_SIGNING_PRIVATE_KEY_PATH = defaultKey;
  }
  environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";
  return environment;
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
let sourceWindowsSignature;
if (windowsInstaller) {
  await access(windowsInstaller);
  if (!basename(windowsInstaller).endsWith("-setup.exe") || !basename(windowsInstaller).includes(version)) {
    throw new Error(`Windows installer must be a ${version} NSIS -setup.exe file`);
  }
  sourceWindowsSignature = `${windowsInstaller}.sig`;
  await rm(sourceWindowsSignature, { force: true });
  run(
    "npm",
    ["run", "tauri", "--", "signer", "sign", windowsInstaller],
    { env: await updaterSigningEnvironment() },
  );
  run("cargo", [
    "run",
    "--quiet",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--example",
    "verify_update_signature",
    "--",
    "src-tauri/tauri.conf.json",
    sourceWindowsSignature,
    windowsInstaller,
  ]);
}
const publishDirectory = join(root, "src-tauri", "target", "release", "publish");
await rm(publishDirectory, { recursive: true, force: true });
await mkdir(publishDirectory, { recursive: true });

const dmg = join(publishDirectory, `DSH.Desktop_${version}_aarch64.dmg`);
const update = join(publishDirectory, `DSH.Desktop_${version}_aarch64.app.tar.gz`);
const signature = `${update}.sig`;
await copyOrLink(sourceDmg, dmg);
await copyOrLink(sourceUpdate, update);
await copyOrLink(sourceSignature, signature);

const publishedWindowsInstaller = windowsInstaller
  ? join(publishDirectory, `DSH.Desktop_${version}_x64-setup.exe`)
  : undefined;
const publishedWindowsSignature = publishedWindowsInstaller
  ? `${publishedWindowsInstaller}.sig`
  : undefined;
if (windowsInstaller && sourceWindowsSignature && publishedWindowsInstaller && publishedWindowsSignature) {
  await copyOrLink(windowsInstaller, publishedWindowsInstaller);
  await copyOrLink(sourceWindowsSignature, publishedWindowsSignature);
}

const signatureContent = (await readFile(signature, "utf8")).trim();
const publishedAt = new Date().toISOString();
const updateUrl = `https://github.com/${repository}/releases/download/${tag}/${basename(update)}`;
const platforms = {
  "darwin-aarch64": {
    signature: signatureContent,
    url: updateUrl,
  },
};
if (publishedWindowsInstaller && publishedWindowsSignature) {
  platforms["windows-x86_64"] = {
    signature: (await readFile(publishedWindowsSignature, "utf8")).trim(),
    url: `https://github.com/${repository}/releases/download/${tag}/${basename(publishedWindowsInstaller)}`,
  };
}
const latest = {
  version,
  notes,
  pub_date: publishedAt,
  platforms,
};
const latestPath = join(publishDirectory, "latest.json");
await writeFile(latestPath, `${JSON.stringify(latest, null, 2)}\n`);

const dmgHash = await sha256(dmg);
const updateHash = await sha256(update);
const windowsHash = publishedWindowsInstaller
  ? await sha256(publishedWindowsInstaller)
  : undefined;
const checksumsPath = join(publishDirectory, "SHA256SUMS.txt");
const checksumLines = [
  `${dmgHash}  ${basename(dmg)}`,
  `${updateHash}  ${basename(update)}`,
];
if (publishedWindowsInstaller && windowsHash) {
  checksumLines.push(`${windowsHash}  ${basename(publishedWindowsInstaller)}`);
}
await writeFile(
  checksumsPath,
  `${checksumLines.join("\n")}\n`,
);

const releaseNotesPath = join(publishDirectory, "release-notes.md");
await writeFile(
  releaseNotesPath,
  `${notes}\n\n` +
    `- Apple Silicon macOS\n` +
    (publishedWindowsInstaller ? `- x64 Windows 10/11\n` : "") +
    `- Requires Node.js 22.19+ (22.x) or 24+\n` +
    `- Reuses an existing DSH or initializes it in a user-level prefix\n` +
    `- macOS is ad-hoc signed and not notarized\n` +
    (publishedWindowsInstaller ? `- Windows is not Authenticode signed\n` : "") +
    `\nmacOS first installation may require:\n\n` +
    "```sh\n" +
    'xattr -dr com.apple.quarantine "/Applications/DSH Desktop.app"\n' +
    "```\n\n" +
    (publishedWindowsInstaller
      ? `On Windows, choose “More info” → “Run anyway” if SmartScreen warns about the unsigned installer.\n\n`
      : "") +
    `DMG SHA-256: \`${dmgHash}\`\n` +
    (windowsHash ? `Windows installer SHA-256: \`${windowsHash}\`\n` : ""),
);

process.stdout.write(`Prepared signed release files in ${publishDirectory}\n`);
process.stdout.write(`DMG SHA-256: ${dmgHash}\n`);
if (windowsHash) process.stdout.write(`Windows installer SHA-256: ${windowsHash}\n`);
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
const releaseAssets = [
  `${dmg}#DSH Desktop ${version} (Apple Silicon)`,
  update,
  signature,
];
if (publishedWindowsInstaller && publishedWindowsSignature) {
  releaseAssets.push(
    `${publishedWindowsInstaller}#DSH Desktop ${version} (Windows x64)`,
    publishedWindowsSignature,
  );
}
releaseAssets.push(latestPath, checksumsPath);
run("gh", [
  "release",
  "create",
  tag,
  ...releaseAssets,
  "--repo",
  repository,
  "--verify-tag",
  "--title",
  `DSH Desktop ${tag}`,
  "--notes-file",
  releaseNotesPath,
  "--latest",
]);
