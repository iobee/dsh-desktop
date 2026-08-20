# DSH Desktop

DSH Desktop 是一个独立、轻量的 macOS 与 Windows 外壳，让 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) 可以像普通应用一样打开。macOS 使用沉浸式窗口，不保留单独的标题栏占位，原生窗口按钮仅在鼠标移入左上角时显示；外壳不修改、复制或 fork 来源仓库的代码，运行内容始终来自 npm 官方包 `@deepseek-ai/dsh`。

产品介绍与下载：[DSH Desktop 官网](https://iobee.github.io/dsh-desktop/)

## 使用方式

### macOS

1. 打开 DMG，把 `DSH Desktop.app` 拖进“应用程序”。
2. 这是未公证的极客向构建。首次运行若被 macOS 拦截，可在终端执行：

   ```sh
   xattr -dr com.apple.quarantine "/Applications/DSH Desktop.app"
   ```

3. 再正常打开应用。Node、npm 和首个可用 DSH 版本都已包含在 App 中，无需预装开发环境。

也可以先尝试打开一次，然后到“系统设置 → 隐私与安全性”选择“仍要打开”。不要全局关闭 Gatekeeper。

### Windows

1. 下载并运行 `DSH.Desktop_<版本>_x64-setup.exe`。安装只写入当前用户目录，不需要管理员权限。
2. 当前构建没有 Authenticode 证书。若 SmartScreen 拦截，选择“更多信息”→“仍要运行”。
3. Node、npm 和首个可用 DSH 版本都已包含在安装包中，无需预装开发环境。

## 两条更新通道

“DSH Desktop → 关于 DSH Desktop”显示桌面外壳与 DSH 运行时的版本、日志入口和 DeepSeek Harness GitHub 链接。“DSH Desktop → 检查更新…”在独立窗口中显示两条更新通道的状态与进度，并用一个按钮同时检查它们。

### DSH 运行时

- 启动只读取本机当前版本，不等待网络。
- 默认跟随 npm `dist-tags.latest`；“检查更新”窗口可开启“加入 DSH Beta”，改为跟随预发布的 `dist-tags.next`。
- 切换通道后立即检查一次；此后的手动检查与每 12 小时后台检查都使用所选通道。
- 退出 Beta 会取消尚未应用的 Beta 更新，但不会把当前正在运行的较新 DSH 自动降级；Desktop 会等 `latest` 追平后再更新。
- 界面打开 60 秒后开始首次后台检查；应用持续运行时也会按该周期继续检查。
- 新版本会安装到独立的版本目录，使用捆绑的 Node/npm 验证版本并完成一次启动冒烟测试。
- 正在运行的版本不会被替换；新版本在下次启动时切换。
- 若新版本启动失败，应用自动退回上一版本。网络或安装失败也不会影响当前版本。
- “DSH Desktop → 检查更新…”可随时强制检查，不受 12 小时间隔或失败重试间隔限制；“重新启动”可应用已就绪的更新。

### DSH Desktop 外壳

- 界面打开 120 秒后，若距离上次成功检查已满 24 小时，后台读取 GitHub Release 的 `latest.json`，不影响启动速度。
- 发现新版本时由用户确认后再下载；Tauri 使用应用内置公钥强制验签，验签通过才会安装。
- 安装完成后自动结束本机 DSH 子进程并重启应用。
- 检查或安装失败不会影响当前应用；失败后最早一小时重试。
- “DSH Desktop → 检查更新…”可随时手动检查。

Windows 与 macOS 使用同一份签名更新清单；应用只会下载与当前系统和架构匹配的更新包。

## 可选终端命令

默认情况下，DSH Desktop 不修改用户 PATH，也不影响终端里已有的 `dsh`。需要在终端复用桌面应用管理的同一套 DSH 时，可选择“DSH Desktop → 安装终端命令…”：

- 若终端中没有 `dsh`，安装名为 `dsh` 的轻量启动命令。
- 若检测到已有 `dsh`，改为安装 `dsh-desktop`，不覆盖、不移动，也不改变原命令的优先级。
- macOS 会同时配置 zsh 的 `.zshrc` 与 fish 的 `conf.d/dsh-desktop.fish`；旧版本写入 `.zprofile` 的受管片段会在重新安装时迁移。
- 该命令始终读取 Desktop 当前生效的隔离运行时，因此会跟随应用从 npm 安装并验证过的 DSH 更新。
- “移除终端命令…”只删除 DSH Desktop 自己创建的命令和 PATH 配置。

Harness 的用户配置和会话仍由上游存放在 `~/.dsh`。外壳自己的版本缓存、可选终端启动器与日志位于 `~/Library/Application Support/com.iobee.dsh-desktop/`。

Windows 上的外壳数据位于 `%APPDATA%\com.iobee.dsh-desktop\`。

## 本机构建

macOS 要求：Apple Silicon Mac、Rust、Xcode Command Line Tools、用于执行准备脚本的 Node，以及更新签名私钥 `~/.tauri/dsh-desktop.key`。

```sh
npm ci
npm run desktop:build
```

`prepare:runtime` 会按构建主机下载 Node 24.19.0 LTS 的官方 macOS arm64 或 Windows x64 发行包，校验 Node 官方 SHA-256，并捆绑 npm 11.19.0，用它安装当时的 `@deepseek-ai/dsh@latest`。macOS 构建会生成 DMG、`DSH Desktop.app.tar.gz` 更新包和对应的 `.sig` 签名，产物位于 `src-tauri/target/release/bundle/`。

Windows 安装包由 `.github/workflows/windows-build.yml` 在 GitHub 的 Windows runner 上原生构建。工作流会先验证源码资源中的运行时，再静默安装生成的 NSIS 包，并从真实安装目录启动一次捆绑 DSH；只有这两层冒烟测试都通过才保存 Actions artifact。

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
