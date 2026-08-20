#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, rm, symlink } from "node:fs/promises";
import { arch, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(await readFile(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const bundleRoot = join(root, "src-tauri", "target", "release", "bundle");
const appName = `${config.productName}.app`;
const sourceApp = join(bundleRoot, "macos", appName);
const architecture = arch() === "arm64" ? "aarch64" : arch();
const outputDir = join(bundleRoot, "dmg");
const output = join(outputDir, `${config.productName}_${config.version}_${architecture}.dmg`);
const stage = await mkdtemp(join(tmpdir(), "dsh-desktop-dmg-"));

try {
  execFileSync(
    "codesign",
    ["--verify", "--deep", "--strict", "--verbose=2", sourceApp],
    { stdio: "inherit" },
  );
  await mkdir(outputDir, { recursive: true });
  await cp(sourceApp, join(stage, appName), { recursive: true, preserveTimestamps: true });
  await symlink("/Applications", join(stage, "Applications"));
  await rm(output, { force: true });
  execFileSync(
    "hdiutil",
    [
      "create",
      "-volname",
      config.productName,
      "-srcfolder",
      stage,
      "-fs",
      "HFS+",
      "-ov",
      "-format",
      "UDBZ",
      "-imagekey",
      "bzip2-level=9",
      output,
    ],
    { stdio: "inherit" },
  );
  process.stdout.write(`Created ${output}\n`);
} finally {
  await rm(stage, { recursive: true, force: true });
}
