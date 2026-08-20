#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { arch, platform, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const READY_PREFIX = "dsh web: http://127.0.0.1:";
const TIMEOUT_MS = 30_000;
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const resources = process.env.DSH_DESKTOP_RESOURCES
  ? resolve(process.env.DSH_DESKTOP_RESOURCES)
  : join(root, "src-tauri", "resources");
const manifest = JSON.parse(await readFile(join(resources, "runtime-manifest.json"), "utf8"));
const hostPlatform = platform();
const hostArch = arch();

if (manifest.platform !== hostPlatform || manifest.arch !== hostArch) {
  throw new Error(
    `Bundled runtime targets ${manifest.platform}-${manifest.arch}, not ${hostPlatform}-${hostArch}`,
  );
}

const node = join(resources, "node", hostPlatform === "win32" ? "node.exe" : "bin/node");
const cli = join(
  resources,
  "bootstrap-runtime",
  "node_modules",
  "@deepseek-ai",
  "dsh",
  "lib",
  "bin.js",
);
const smokeHome = await mkdtemp(join(tmpdir(), "dsh-desktop-runtime-"));
const child = spawn(node, [cli, "web", "--port", "0", "--no-open"], {
  cwd: smokeHome,
  detached: hostPlatform !== "win32",
  env: {
    ...process.env,
    DSH_DESKTOP: "1",
    DSH_HOME: smokeHome,
    NODE_ENV: "production",
  },
  stdio: ["ignore", "pipe", "pipe"],
  windowsHide: true,
});

let output = "";
let settled = false;

function terminate() {
  if (child.exitCode !== null) return;
  if (hostPlatform === "win32") {
    spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
  } else {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      child.kill("SIGTERM");
    }
  }
}

function finish(error) {
  if (settled) return;
  settled = true;
  terminate();
  if (error) {
    process.exitCode = 1;
    process.stderr.write(`${error.message}\n${output}`);
  }
}

const timeout = setTimeout(() => {
  finish(new Error(`dsh web did not become ready within ${TIMEOUT_MS}ms`));
}, TIMEOUT_MS);

child.stdout.setEncoding("utf8");
child.stderr.setEncoding("utf8");
child.stdout.on("data", (chunk) => {
  output += chunk;
  const readyLine = output.split(/\r?\n/u).find((line) => line.startsWith(READY_PREFIX));
  if (!readyLine) return;
  clearTimeout(timeout);
  process.stdout.write(`${readyLine}\n`);
  finish();
});
child.stderr.on("data", (chunk) => {
  output += chunk;
});
child.on("error", (error) => {
  clearTimeout(timeout);
  finish(error);
});
child.on("exit", (code, signal) => {
  if (settled) return;
  clearTimeout(timeout);
  finish(new Error(`dsh web exited before readiness (code ${code}, signal ${signal})`));
});

await new Promise((resolve) => child.once("close", resolve));
await rm(smokeHome, { recursive: true, force: true });
if (process.exitCode) process.exit(process.exitCode);
