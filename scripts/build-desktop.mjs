#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { access } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const environment = { ...process.env };

if (!environment.TAURI_SIGNING_PRIVATE_KEY) {
  const defaultKey = join(homedir(), ".tauri", "dsh-desktop.key");
  try {
    await access(defaultKey);
  } catch {
    throw new Error(
      `Updater signing key not found at ${defaultKey}. See README.md before building a release.`,
    );
  }
  environment.TAURI_SIGNING_PRIVATE_KEY = defaultKey;
}
environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";

execFileSync("npm", ["run", "prepare:runtime"], {
  cwd: root,
  env: environment,
  stdio: "inherit",
});
execFileSync("npm", ["run", "tauri", "--", "build"], {
  cwd: root,
  env: environment,
  stdio: "inherit",
});
execFileSync("node", ["scripts/package-dmg.mjs"], {
  cwd: root,
  env: environment,
  stdio: "inherit",
});
