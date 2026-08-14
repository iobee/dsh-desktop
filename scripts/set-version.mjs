#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const version = process.argv[2];
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/u;
if (!version || !semver.test(version)) {
  throw new Error("Usage: npm run version:set -- <semver>");
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

async function updateJson(relative, update) {
  const path = join(root, relative);
  const value = JSON.parse(await readFile(path, "utf8"));
  update(value);
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

await updateJson("package.json", (value) => {
  value.version = version;
});
await updateJson("package-lock.json", (value) => {
  value.version = version;
  value.packages[""].version = version;
});
await updateJson("src-tauri/tauri.conf.json", (value) => {
  value.version = version;
});

const cargoTomlPath = join(root, "src-tauri", "Cargo.toml");
const cargoToml = await readFile(cargoTomlPath, "utf8");
const updatedCargoToml = cargoToml.replace(
  /(\[package\][\s\S]*?\nversion = ")[^"]+("\n)/u,
  `$1${version}$2`,
);
if (updatedCargoToml === cargoToml) throw new Error("Could not update Cargo.toml version");
await writeFile(cargoTomlPath, updatedCargoToml);

const cargoLockPath = join(root, "src-tauri", "Cargo.lock");
const cargoLock = await readFile(cargoLockPath, "utf8");
const updatedCargoLock = cargoLock.replace(
  /(\[\[package\]\]\nname = "dsh-desktop"\nversion = ")[^"]+("\n)/u,
  `$1${version}$2`,
);
if (updatedCargoLock === cargoLock) throw new Error("Could not update Cargo.lock version");
await writeFile(cargoLockPath, updatedCargoLock);

process.stdout.write(`Set DSH Desktop version to ${version}\n`);
