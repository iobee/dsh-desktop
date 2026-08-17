import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import { messageOf, readVersionInfo, tauriRuntime } from "./version-info.ts";

const githubUrl = "https://github.com/deepseek-ai/deepseek-harness";

const desktopVersion = element<HTMLElement>("desktop-version");
const dshVersion = element<HTMLElement>("dsh-version");
const dshGithub = element<HTMLAnchorElement>("dsh-github");
const openLogs = element<HTMLButtonElement>("open-logs");
const toast = element<HTMLElement>("toast");

function element<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing About element: ${id}`);
  return value as T;
}

async function refresh(): Promise<void> {
  try {
    const info = await readVersionInfo();
    desktopVersion.textContent = info.desktopVersion;
    dshVersion.textContent = info.runtime.currentVersion;
  } catch (reason) {
    showToast(`无法读取版本信息：${messageOf(reason)}`);
  }
}

function showToast(message: string): void {
  toast.textContent = message;
  toast.hidden = false;
  window.setTimeout(() => {
    toast.hidden = true;
  }, 3_500);
}

dshGithub.addEventListener("click", (event) => {
  if (!tauriRuntime) return;
  event.preventDefault();
  void openUrl(githubUrl).catch((reason) => showToast(messageOf(reason)));
});

openLogs.addEventListener("click", () => {
  if (tauriRuntime) {
    void invoke("open_logs").catch((reason) => showToast(messageOf(reason)));
  } else {
    showToast("安装版会在 Finder 中打开日志目录");
  }
});

void refresh();
