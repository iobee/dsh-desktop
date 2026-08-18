import { invoke } from "@tauri-apps/api/core";

import {
  type AppUpdateSnapshot,
  messageOf,
  previewInfo,
  readVersionInfo,
  type RuntimeUpdateSnapshot,
  tauriRuntime,
  type VersionInfo,
} from "./version-info.ts";

interface StatusPresentation {
  label: string;
  detail: string;
  tone: "neutral" | "success" | "accent" | "danger";
  busy: boolean;
  showProgress: boolean;
  progress: number | null;
}

const dshVersion = element<HTMLElement>("dsh-version");
const desktopVersion = element<HTMLElement>("desktop-version");
const dshStatus = element<HTMLElement>("dsh-status");
const desktopStatus = element<HTMLElement>("desktop-status");
const dshIndicator = element<HTMLImageElement>("dsh-indicator");
const desktopIndicator = element<HTMLImageElement>("desktop-indicator");
const dshDetail = element<HTMLElement>("dsh-detail");
const desktopDetail = element<HTMLElement>("desktop-detail");
const dshProgressWrap = element<HTMLElement>("dsh-progress-wrap");
const desktopProgressWrap = element<HTMLElement>("desktop-progress-wrap");
const dshProgress = element<HTMLProgressElement>("dsh-progress");
const desktopProgress = element<HTMLProgressElement>("desktop-progress");
const checkAll = element<HTMLButtonElement>("check-all");
const toast = element<HTMLElement>("toast");

function element<T extends HTMLElement>(id: string): T {
  const value = document.getElementById(id);
  if (!value) throw new Error(`Missing Updates element: ${id}`);
  return value as T;
}

function runtimePresentation(update: RuntimeUpdateSnapshot): StatusPresentation {
  const target = update.targetVersion;
  switch (update.phase) {
    case "checking":
      return status("正在检查", update.detail, "accent", true, false, null);
    case "current":
      return status("已是最新版本", update.detail, "success", false, false, null);
    case "available":
      return status(
        target ? `${target} 可用` : "发现新版本",
        update.detail,
        "accent",
        false,
        false,
        null,
      );
    case "installing":
      return status(
        target ? `正在安装 ${target}` : "正在安装",
        null,
        "accent",
        true,
        false,
        null,
      );
    case "verifying":
      return status("正在验证", update.detail, "accent", true, false, null);
    case "ready":
      return status(
        target ? `${target} 已就绪` : "更新已就绪",
        update.detail,
        "accent",
        false,
        false,
        null,
      );
    case "error":
      return status(errorLabel(update.detail), update.detail, "danger", false, false, null);
    case "idle":
      return status("当前版本", update.detail, "neutral", false, false, null);
  }
}

function desktopPresentation(update: AppUpdateSnapshot): StatusPresentation {
  const target = update.targetVersion;
  switch (update.phase) {
    case "checking":
      return status("正在检查", update.detail, "accent", true, false, null);
    case "current":
      return status("已是最新版本", update.detail, "success", false, false, null);
    case "available":
      return status(
        target ? `${target} 可用` : "发现新版本",
        update.detail,
        "accent",
        false,
        false,
        null,
      );
    case "downloading":
      return status(
        update.progress === null ? "正在下载" : `正在下载 ${update.progress}%`,
        update.detail,
        "accent",
        true,
        true,
        update.progress,
      );
    case "installing":
      return status("正在安装", update.detail, "accent", true, true, update.progress);
    case "error":
      return status(errorLabel(update.detail), update.detail, "danger", false, false, null);
    case "idle":
      return status("当前版本", update.detail, "neutral", false, false, null);
  }
}

function status(
  label: string,
  detail: string | null,
  tone: StatusPresentation["tone"],
  busy: boolean,
  showProgress: boolean,
  progress: number | null,
): StatusPresentation {
  const normalizedDetail = detail === label
    ? ""
    : tone === "danger"
      ? friendlyErrorDetail(detail)
      : detail ?? "";
  return {
    label,
    detail: normalizedDetail,
    tone,
    busy,
    showProgress,
    progress,
  };
}

function errorLabel(detail: string | null): string {
  const value = detail?.toLowerCase() ?? "";
  return value.includes("install") || value.includes("安装") || value.includes("下载")
    ? "安装失败"
    : "检查失败";
}

function friendlyErrorDetail(detail: string | null): string {
  const value = detail?.toLowerCase() ?? "";
  if (
    detail &&
    (value.includes("无法连接") || value.includes("未通过校验") || value.includes("稍后重试"))
  ) {
    return detail.replace(/[。.]+$/u, "");
  }
  if (
    value.includes("sending request") ||
    value.includes("connect") ||
    value.includes("timed out") ||
    value.includes("timeout") ||
    value.includes("dns") ||
    value.includes("econn")
  ) {
    return "无法连接更新服务，请检查网络后重试";
  }
  if (value.includes("signature") || value.includes("签名") || value.includes("json")) {
    return "更新信息未通过校验，已停止安装";
  }
  return "请稍后重试，详细信息已写入日志";
}

function render(info: VersionInfo): void {
  desktopVersion.textContent = info.desktopVersion;
  dshVersion.textContent = info.runtime.currentVersion;

  const desktop = desktopPresentation(info.desktopUpdate);
  const runtime = runtimePresentation(info.runtime.update);
  renderStatus(
    desktop,
    desktopStatus,
    desktopIndicator,
    desktopDetail,
    desktopProgressWrap,
    desktopProgress,
  );
  renderStatus(
    runtime,
    dshStatus,
    dshIndicator,
    dshDetail,
    dshProgressWrap,
    dshProgress,
  );

  checkAll.disabled = desktop.busy || runtime.busy;
  checkAll.textContent = checkAll.disabled
    ? "正在更新…"
    : info.desktopUpdate.phase === "available" || info.runtime.update.phase === "available"
      ? "继续更新"
      : "检查更新";
}

function renderStatus(
  presentation: StatusPresentation,
  statusElement: HTMLElement,
  indicator: HTMLImageElement,
  detailElement: HTMLElement,
  progressWrap: HTMLElement,
  progressElement: HTMLProgressElement,
): void {
  statusElement.textContent = presentation.label;
  statusElement.dataset.tone = presentation.tone;
  indicator.hidden = presentation.tone === "neutral";
  indicator.dataset.tone = presentation.tone;
  indicator.src = presentation.tone === "success"
    ? "/src/assets/status-current.png"
    : "/src/assets/status-active.png";
  detailElement.textContent = presentation.detail;
  detailElement.title = presentation.detail;
  detailElement.hidden = presentation.detail.length === 0;
  progressWrap.hidden = !presentation.showProgress;
  if (presentation.progress === null) {
    progressElement.removeAttribute("value");
  } else {
    progressElement.value = presentation.progress;
  }
}

async function refresh(): Promise<void> {
  try {
    render(await readVersionInfo());
  } catch (reason) {
    showToast(`无法读取更新状态：${messageOf(reason)}`);
  }
}

async function requestAllUpdates(): Promise<void> {
  if (!tauriRuntime) {
    simulatePreviewCheck();
    return;
  }

  checkAll.disabled = true;
  checkAll.textContent = "正在检查…";
  try {
    const [runtimeStarted, desktopStarted] = await Promise.all([
      invoke<boolean>("check_dsh_update"),
      invoke<boolean>("check_app_update"),
    ]);
    if (!runtimeStarted && !desktopStarted) showToast("更新检查已经在进行中");
    await refresh();
  } catch (reason) {
    showToast(`无法开始更新检查：${messageOf(reason)}`);
    await refresh();
  }
}

function simulatePreviewCheck(): void {
  previewInfo.runtime.update = {
    phase: "checking",
    targetVersion: null,
    detail: "正在查询 npm 最新版本",
  };
  previewInfo.desktopUpdate = {
    phase: "checking",
    targetVersion: null,
    progress: null,
    detail: "正在检查 GitHub Release",
  };
  render(previewInfo);
  window.setTimeout(() => {
    previewInfo.runtime.update = {
      phase: "current",
      targetVersion: null,
      detail: "已是最新版本",
    };
    previewInfo.desktopUpdate = {
      phase: "current",
      targetVersion: null,
      progress: null,
      detail: "已是最新版本",
    };
    render(previewInfo);
  }, 900);
}

function showToast(message: string): void {
  toast.textContent = message;
  toast.hidden = false;
  window.setTimeout(() => {
    toast.hidden = true;
  }, 3_500);
}

if (!tauriRuntime) {
  const previewState = new URLSearchParams(window.location.search).get("state");
  if (previewState === "downloading") {
    previewInfo.desktopUpdate = {
      phase: "downloading",
      targetVersion: "0.1.6",
      progress: 68,
      detail: "安装完成后将自动重启",
    };
  } else if (previewState === "problem") {
    previewInfo.desktopUpdate = {
      phase: "error",
      targetVersion: null,
      progress: null,
      detail: "GitHub 更新检查失败：error sending request for url (https://github.com/iobee/dsh-desktop/releases/latest/download/latest.json)",
    };
    previewInfo.runtime.update = {
      phase: "installing",
      targetVersion: "0.1.0-rc.7",
      detail: "正在安装 DSH 0.1.0-rc.7",
    };
  }
}

checkAll.addEventListener("click", () => void requestAllUpdates());

void refresh();
if (tauriRuntime) window.setInterval(() => void refresh(), 600);
