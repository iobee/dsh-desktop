# DSH Desktop

DSH Desktop 是一个独立、轻量的 macOS 与 Windows 外壳，让 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) 可以像普通应用一样打开。macOS 使用沉浸式窗口，不保留单独的标题栏占位，原生窗口按钮仅在鼠标移入左上角时显示；外壳不修改、复制或 fork 来源仓库的代码，运行内容始终来自 npm 官方包 `@deepseek-ai/dsh`。

## 使用方式

### macOS

1. 打开 DMG，把 `DSH Desktop.app` 拖进“应用程序”。
2. 这是未公证的极客向构建。首次运行若被 macOS 拦截，可在终端执行：

   ```sh
   xattr -dr com.apple.quarantine "/Applications/DSH Desktop.app"
   ```

3. 再正常打开应用。Node、npm 和首个可用 dsh 版本都已包含在 App 中，无需预装开发环境。

也可以先尝试打开一次，然后到“系统设置 → 隐私与安全性”选择“仍要打开”。不要全局关闭 Gatekeeper。

### Windows

1. 下载并运行 `DSH.Desktop_<版本>_x64-setup.exe`。安装只写入当前用户目录，不需要管理员权限。
2. 当前构建没有 Authenticode 证书。若 SmartScreen 拦截，选择“更多信息”→“仍要运行”。
3. Node、npm 和首个可用 dsh 版本都已包含在安装包中，无需预装开发环境。

## 两条更新通道

### dsh 运行时

- 启动只读取本机当前版本，不等待网络。
- 界面打开 60 秒后，若距离上次成功检查已满 24 小时，后台读取 npm `dist-tags.latest`。
- 新版本会安装到独立的版本目录，使用捆绑的 Node/npm 验证版本并完成一次启动冒烟测试。
- 正在运行的版本不会被替换；新版本在下次启动时切换。
- 若新版本启动失败，应用自动退回上一版本。网络或安装失败也不会影响当前版本。
- “DSH Desktop → 检查 dsh 更新…”可手动检查；“重新启动”可应用已就绪的更新。

### DSH Desktop 外壳

- 界面打开 120 秒后，若距离上次成功检查已满 24 小时，后台读取 GitHub Release 的 `latest.json`，不影响启动速度。
- 发现新版本时由用户确认后再下载；Tauri 使用应用内置公钥强制验签，验签通过才会安装。
- 安装完成后自动结束本机 dsh 子进程并重启应用。
- 检查或安装失败不会影响当前应用；失败后最早一小时重试。
- “DSH Desktop → 检查 DSH Desktop 更新…”可随时手动检查。

Windows 与 macOS 使用同一份签名更新清单；应用只会下载与当前系统和架构匹配的更新包。

Harness 的用户配置和会话仍由上游存放在 `~/.dsh`。外壳自己的版本缓存与日志位于 `~/Library/Application Support/com.iobee.dsh-desktop/`。

Windows 上的外壳数据位于 `%APPDATA%\com.iobee.dsh-desktop\`。

## 本机构建

macOS 要求：Apple Silicon Mac、Rust、Xcode Command Line Tools、用于执行准备脚本的 Node，以及更新签名私钥 `~/.tauri/dsh-desktop.key`。

```sh
npm ci
npm run desktop:build
```

`prepare:runtime` 会按构建主机下载 Node 24.19.0 LTS 的官方 macOS arm64 或 Windows x64 发行包，校验 Node 官方 SHA-256，并捆绑 npm 11.19.0，用它安装当时的 `@deepseek-ai/dsh@latest`。macOS 构建会生成 DMG、`DSH Desktop.app.tar.gz` 更新包和对应的 `.sig` 签名，产物位于 `src-tauri/target/release/bundle/`。

Windows 安装包由 `.github/workflows/windows-build.yml` 在 GitHub 的 Windows runner 上原生构建。工作流会先启动一次捆绑的 dsh 做冒烟测试，再生成当前用户安装模式的 NSIS `-setup.exe`，并作为 Actions artifact 保存 14 天。

## 发布新版本

更新签名私钥不能提交到仓库，也不能随意重新生成；丢失它后，已安装用户将无法验证后续更新。公钥已经固定在 `tauri.conf.json`，私钥默认从 `~/.tauri/dsh-desktop.key` 读取并应另行安全备份。

以后发布时先同步版本号并提交到 `main`：

```sh
npm run version:set -- 0.1.5
npm run release:verify -- v0.1.5
git add -A
git commit -m "Release v0.1.5"
```

等待 `Windows build` 工作流通过，再把其 artifact 下载到本机：

```sh
gh run list --workflow "Windows build" --branch main
gh run download <run-id> --name dsh-desktop-windows-x64 --dir dist-windows
```

确认提交和 Windows 安装包无误后运行：

```sh
npm run release:publish -- --windows-installer "dist-windows/DSH Desktop_0.1.5_x64-setup.exe" "本次更新说明"
```

该命令会在本机运行测试、准备最新 npm dsh、ad-hoc 签名 macOS 应用，并用同一把更新私钥为 macOS 更新包和 Windows 安装包签名。随后它生成同时包含 `darwin-aarch64` 与 `windows-x86_64` 的 `latest.json`，推送 `main` 和版本标签，并通过已登录的 `gh` 创建 GitHub Release。私钥全程留在本机，不会交给 GitHub Actions。只想验证完整打包链路而不推送时使用：

```sh
npm run release:publish -- --dry-run --windows-installer "dist-windows/DSH Desktop_0.1.5_x64-setup.exe"
```

当前发行物没有 Apple Developer ID、公证或 Windows Authenticode 签名，适合小范围极客用户；Tauri 更新包仍使用项目自己的私钥强制验签。以后获得平台证书时，系统信任签名可以与 npm dsh 更新继续保持两条独立通道。

## 许可与来源

本外壳使用 MIT License。DeepSeek Harness 及其依赖保留各自的许可与版权；上游 npm 包被原样安装为运行时依赖。
