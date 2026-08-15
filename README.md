# DSH Desktop

DSH Desktop 是一个独立、轻量的 macOS 外壳，让 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) 可以像普通应用一样打开。它使用沉浸式 macOS 窗口，不保留单独的标题栏占位；也不修改、复制或 fork 来源仓库的代码，运行内容始终来自 npm 官方包 `@deepseek-ai/dsh`。

## 使用方式

1. 打开 DMG，把 `DSH Desktop.app` 拖进“应用程序”。
2. 这是未公证的极客向构建。首次运行若被 macOS 拦截，可在终端执行：

   ```sh
   xattr -dr com.apple.quarantine "/Applications/DSH Desktop.app"
   ```

3. 再正常打开应用。Node、npm 和首个可用 dsh 版本都已包含在 App 中，无需预装开发环境。

也可以先尝试打开一次，然后到“系统设置 → 隐私与安全性”选择“仍要打开”。不要全局关闭 Gatekeeper。

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

`v0.1.0` 没有内置外壳更新器，因此需要手动安装一次带更新器的后续版本；之后的外壳版本可以在应用内更新。

Harness 的用户配置和会话仍由上游存放在 `~/.dsh`。外壳自己的版本缓存与日志位于 `~/Library/Application Support/com.iobee.dsh-desktop/`。

## 本机构建

要求：Apple Silicon Mac、Rust、Xcode Command Line Tools、用于执行准备脚本的 Node，以及更新签名私钥 `~/.tauri/dsh-desktop.key`。

```sh
npm ci
npm run desktop:build
```

`prepare:runtime` 固定下载 Node 24.19.0 LTS 的官方 arm64 macOS 发行包，校验 Node 官方 SHA-256，并捆绑 npm 11.19.0，用它安装当时 `@deepseek-ai/dsh@latest`。构建会生成 DMG、`DSH Desktop.app.tar.gz` 更新包和对应的 `.sig` 签名，产物位于 `src-tauri/target/release/bundle/`。

## 发布新版本

更新签名私钥不能提交到仓库，也不能随意重新生成；丢失它后，已安装用户将无法验证后续更新。公钥已经固定在 `tauri.conf.json`，私钥默认从 `~/.tauri/dsh-desktop.key` 读取并应另行安全备份。

以后发布时先同步版本号并提交到 `main`：

```sh
npm run version:set -- 0.1.3
npm run release:verify -- v0.1.3
git add -A
git commit -m "Release v0.1.3"
```

确认提交无误后运行：

```sh
npm run release:publish -- "本次更新说明"
```

该命令会在本机运行测试、准备最新 npm dsh、ad-hoc 签名应用、生成更新签名与 `latest.json`，然后推送 `main` 和版本标签，并通过已登录的 `gh` 创建 GitHub Release。私钥全程留在本机，不会交给第三方 GitHub Action。只想验证完整打包链路而不推送时使用：

```sh
npm run release:publish -- --dry-run
```

当前发行物没有 Developer ID 签名或 Apple 公证，适合小范围极客用户；以后若获得证书，外壳签名更新可以与 npm dsh 更新保持两条独立通道。

## 许可与来源

本外壳使用 MIT License。DeepSeek Harness 及其依赖保留各自的许可与版权；上游 npm 包被原样安装为运行时依赖。
