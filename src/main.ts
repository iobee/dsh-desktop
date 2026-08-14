import { invoke } from "@tauri-apps/api/core";

interface StartupInfo {
  url: string;
  dshVersion: string;
}

const status = document.querySelector<HTMLElement>("#status");
const hint = document.querySelector<HTMLElement>("#hint");
const spinner = document.querySelector<HTMLElement>("#spinner");
const error = document.querySelector<HTMLElement>("#error");
const actions = document.querySelector<HTMLElement>("#actions");
const retry = document.querySelector<HTMLButtonElement>("#retry");
const logs = document.querySelector<HTMLButtonElement>("#logs");

function textOf(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

async function launch(): Promise<void> {
  if (!status || !hint || !spinner || !error || !actions) return;

  status.textContent = "正在启动本机服务…";
  hint.textContent = "直接使用已安装版本；更新检查不会阻塞启动。";
  spinner.hidden = false;
  error.hidden = true;
  actions.hidden = true;

  const slowMessage = window.setTimeout(() => {
    hint.textContent = "首次启动可能需要几秒，之后会直接使用本机缓存。";
  }, 4_000);

  try {
    const result = await invoke<StartupInfo>("bootstrap");
    window.clearTimeout(slowMessage);
    status.textContent = `dsh ${result.dshVersion} 已就绪`;
    hint.textContent = "正在打开…";
    window.location.replace(result.url);
  } catch (reason) {
    window.clearTimeout(slowMessage);
    status.textContent = "启动失败";
    hint.textContent = "当前版本没有被替换，可以查看日志后重试。";
    spinner.hidden = true;
    error.textContent = textOf(reason);
    error.hidden = false;
    actions.hidden = false;
  }
}

retry?.addEventListener("click", () => void launch());
logs?.addEventListener("click", () => void invoke("open_logs"));
void launch();
