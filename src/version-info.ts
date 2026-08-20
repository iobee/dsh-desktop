import { invoke, isTauri } from "@tauri-apps/api/core";

export type RuntimeUpdatePhase =
  | "idle"
  | "checking"
  | "current"
  | "ahead"
  | "installing"
  | "verifying"
  | "ready"
  | "error";

export type RuntimeUpdateChannel = "latest" | "next";

export type AppUpdatePhase =
  | "idle"
  | "checking"
  | "current"
  | "available"
  | "downloading"
  | "installing"
  | "error";

export interface RuntimeUpdateSnapshot {
  phase: RuntimeUpdatePhase;
  targetVersion: string | null;
  detail: string | null;
}

export interface RuntimeSnapshot {
  currentVersion: string;
  pendingVersion: string | null;
  nodeVersion: string;
  npmVersion: string;
  updateChannel: RuntimeUpdateChannel;
  update: RuntimeUpdateSnapshot;
}

export interface AppUpdateSnapshot {
  phase: AppUpdatePhase;
  targetVersion: string | null;
  progress: number | null;
  detail: string | null;
}

export interface VersionInfo {
  desktopVersion: string;
  runtime: RuntimeSnapshot;
  desktopUpdate: AppUpdateSnapshot;
}

export const tauriRuntime = isTauri();

export const previewInfo: VersionInfo = {
  desktopVersion: "0.1.5",
  runtime: {
    currentVersion: "0.1.0-rc.6",
    pendingVersion: null,
    nodeVersion: "24.19.0",
    npmVersion: "11.19.0",
    updateChannel: "latest",
    update: {
      phase: "current",
      targetVersion: null,
      detail: "已是最新版本",
    },
  },
  desktopUpdate: {
    phase: "current",
    targetVersion: null,
    progress: null,
    detail: "已是最新版本",
  },
};

/** Reads the current bundled and installed version state. */
export async function readVersionInfo(): Promise<VersionInfo> {
  return tauriRuntime ? invoke<VersionInfo>("get_about_info") : previewInfo;
}

/** Converts an unknown command failure into user-facing text. */
export function messageOf(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}
