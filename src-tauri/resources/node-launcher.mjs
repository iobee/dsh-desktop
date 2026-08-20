import { pathToFileURL } from "node:url";

const entry = process.env.DSH_DESKTOP_NODE_ENTRY;
if (!entry) {
  throw new Error("DSH_DESKTOP_NODE_ENTRY is required");
}

const cwd = process.env.DSH_DESKTOP_NODE_CWD;
if (cwd) process.chdir(cwd);

process.argv[1] = entry;
await import(pathToFileURL(entry).href);
