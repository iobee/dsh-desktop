#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME;
if (!tag) throw new Error("A release tag is required");

const packageJson = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
const tauriConfig = JSON.parse(
  await readFile(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const cargoToml = await readFile(join(root, "src-tauri", "Cargo.toml"), "utf8");
const cargoVersion = cargoToml.match(/\[package\][\s\S]*?\nversion = "([^"]+)"\n/u)?.[1];
const versions = [packageJson.version, tauriConfig.version, cargoVersion];

if (versions.some((version) => version !== versions[0])) {
  throw new Error(`Version files disagree: ${versions.join(", ")}`);
}
if (tag !== `v${versions[0]}`) {
  throw new Error(`Tag ${tag} does not match app version v${versions[0]}`);
}

process.stdout.write(`Release ${tag} matches all version files\n`);
