#!/usr/bin/env node

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { arch, platform } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const NODE_VERSION = "24.19.0";
const NPM_VERSION = "11.19.0";
const RUNTIME_LAYOUT_VERSION = 4;
const DSH_PACKAGE = "@deepseek-ai/dsh";
const DSH_TAG = "latest";
const DSH_INSTALL_SCRIPTS = [
  "@deepseek-ai/dsh-subprocess-local",
  "@google/genai",
  "koffi",
  "node-pty",
  "protobufjs",
];
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const resources = join(root, "src-tauri", "resources");
const nodeDir = join(resources, "node");
const npmDir = join(resources, "npm");
const runtimeDir = join(resources, "bootstrap-runtime");
const manifestPath = join(resources, "runtime-manifest.json");
const downloads = join(resources, ".downloads");
const hostPlatform = platform();
const hostArch = arch();
const hostNpmCli = process.env.npm_execpath;
if (!hostNpmCli) throw new Error("prepare:runtime must be run through npm");
const target = (() => {
  if (hostPlatform === "darwin" && hostArch === "arm64") {
    return {
      archiveName: `node-v${NODE_VERSION}-darwin-arm64.tar.gz`,
      extractedName: `node-v${NODE_VERSION}-darwin-arm64`,
      nodeExecutable: join("bin", "node"),
      retainedNodePtyPrebuild: "darwin-arm64",
    };
  }
  if (hostPlatform === "win32" && hostArch === "x64") {
    return {
      archiveName: `node-v${NODE_VERSION}-win-x64.zip`,
      extractedName: `node-v${NODE_VERSION}-win-x64`,
      nodeExecutable: "node.exe",
      retainedNodePtyPrebuild: "win32-x64",
    };
  }
  throw new Error(
    `This bootstrap supports Apple Silicon macOS and x64 Windows, got ${hostPlatform}-${hostArch}`,
  );
})();

async function preserveResourceDirectories() {
  await Promise.all(
    [nodeDir, npmDir, runtimeDir].map((directory) => writeFile(join(directory, ".gitkeep"), "\n")),
  );
}

const expectedManifest = existsSync(manifestPath)
  ? JSON.parse(await readFile(manifestPath, "utf8"))
  : undefined;
if (
  expectedManifest?.nodeVersion === NODE_VERSION
  && expectedManifest?.npmVersion === NPM_VERSION
  && expectedManifest?.runtimeLayoutVersion === RUNTIME_LAYOUT_VERSION
  && expectedManifest?.platform === hostPlatform
  && expectedManifest?.arch === hostArch
  && existsSync(join(nodeDir, target.nodeExecutable))
  && existsSync(join(npmDir, "node_modules", "npm", "bin", "npm-cli.js"))
  && existsSync(join(runtimeDir, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js"))
) {
  await preserveResourceDirectories();
  process.stdout.write(
    `Runtime already prepared: Node ${NODE_VERSION}, npm ${NPM_VERSION}, dsh ${expectedManifest.dshVersion}\n`,
  );
  process.exit(0);
}

await mkdir(downloads, { recursive: true });
const archiveName = target.archiveName;
const archive = join(downloads, archiveName);
const baseUrl = `https://nodejs.org/dist/v${NODE_VERSION}`;

function download(url, target) {
  process.stdout.write(`Downloading ${url}\n`);
  execFileSync(hostPlatform === "win32" ? "curl.exe" : "curl", [
    "--fail",
    "--location",
    "--retry",
    "3",
    "--connect-timeout",
    "20",
    "--max-time",
    "300",
    "--output",
    target,
    url,
  ], { stdio: "inherit" });
}

if (!existsSync(archive)) download(`${baseUrl}/${archiveName}`, archive);
const sumsPath = join(downloads, `SHASUMS256-${NODE_VERSION}.txt`);
download(`${baseUrl}/SHASUMS256.txt`, sumsPath);
const sums = await readFile(sumsPath, "utf8");
const expected = sums
  .split("\n")
  .find((line) => line.endsWith(`  ${archiveName}`))
  ?.split(/\s+/u)[0];
if (!expected) throw new Error(`No checksum published for ${archiveName}`);
const actual = createHash("sha256").update(await readFile(archive)).digest("hex");
if (actual !== expected) throw new Error(`Checksum mismatch for ${archiveName}`);

const extractRoot = join(downloads, `extract-${process.pid}`);
await rm(extractRoot, { recursive: true, force: true });
await mkdir(extractRoot, { recursive: true });
execFileSync("tar", [hostPlatform === "win32" ? "-xf" : "-xzf", archive, "-C", extractRoot]);
await rm(nodeDir, { recursive: true, force: true });
await rename(join(extractRoot, target.extractedName), nodeDir);
await rm(extractRoot, { recursive: true, force: true });
for (const relative of ["include", "lib", "share", "node_modules", "README.md", "CHANGELOG.md"]) {
  await rm(join(nodeDir, relative), { recursive: true, force: true });
}
for (const command of ["corepack", "corepack.cmd", "npm", "npm.cmd", "npx", "npx.cmd"]) {
  const commandPath = hostPlatform === "win32" ? join(nodeDir, command) : join(nodeDir, "bin", command);
  await rm(commandPath, { force: true });
}

await rm(runtimeDir, { recursive: true, force: true });
await mkdir(runtimeDir, { recursive: true });
await writeFile(
  join(runtimeDir, "package.json"),
  `${JSON.stringify(
    {
      private: true,
      dependencies: { [DSH_PACKAGE]: DSH_TAG },
      allowScripts: Object.fromEntries(DSH_INSTALL_SCRIPTS.map((name) => [name, true])),
    },
    null,
    2,
  )}\n`,
);
const bundledNode = join(nodeDir, target.nodeExecutable);
await rm(npmDir, { recursive: true, force: true });
await mkdir(npmDir, { recursive: true });
await writeFile(
  join(npmDir, "package.json"),
  `${JSON.stringify({ private: true, dependencies: { npm: NPM_VERSION } }, null, 2)}\n`,
);
execFileSync(
  process.execPath,
  [
    hostNpmCli,
    "install",
    "--omit=dev",
    "--save-exact",
    "--no-audit",
    "--no-fund",
    "--package-lock=false",
  ],
  { cwd: npmDir, stdio: "inherit", env: { ...process.env, npm_config_update_notifier: "false" } },
);
const npmCli = join(npmDir, "node_modules", "npm", "bin", "npm-cli.js");
execFileSync(
  bundledNode,
  [
    npmCli,
    "install",
    "--omit=dev",
    "--save-exact",
    "--no-audit",
    "--no-fund",
    "--package-lock=false",
  ],
  {
    cwd: runtimeDir,
    stdio: "inherit",
    env: {
      ...process.env,
      npm_config_update_notifier: "false",
      npm_config_strict_allow_scripts: "true",
    },
  },
);
const nodePtyPrebuilds = join(runtimeDir, "node_modules", "node-pty", "prebuilds");
for (const prebuild of await readdir(nodePtyPrebuilds)) {
  if (prebuild === target.retainedNodePtyPrebuild) continue;
  await rm(join(nodePtyPrebuilds, prebuild), {
    recursive: true,
    force: true,
  });
}

const dshManifest = JSON.parse(
  await readFile(
    join(runtimeDir, "node_modules", "@deepseek-ai", "dsh", "package.json"),
    "utf8",
  ),
);
execFileSync(
  bundledNode,
  [join(runtimeDir, "node_modules", "@deepseek-ai", "dsh", "lib", "bin.js"), "--version"],
  { stdio: "inherit" },
);
await writeFile(
  manifestPath,
  `${JSON.stringify(
    {
      nodeVersion: NODE_VERSION,
      npmVersion: NPM_VERSION,
      runtimeLayoutVersion: RUNTIME_LAYOUT_VERSION,
      dshVersion: dshManifest.version,
      platform: hostPlatform,
      arch: hostArch,
    },
    null,
    2,
  )}\n`,
);
await preserveResourceDirectories();
process.stdout.write(
  `Prepared Node ${NODE_VERSION}, npm ${NPM_VERSION}, and ${DSH_PACKAGE} ${dshManifest.version}\n`,
);
