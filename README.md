# DSH Desktop

DSH Desktop 是一个独立、轻量的 macOS 外壳，让 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) 可以像普通应用一样打开。它不修改、复制或 fork 来源仓库的代码；运行内容始终来自 npm 官方包 `@deepseek-ai/dsh`。

## 使用方式

1. 打开 DMG，把 `DSH Desktop.app` 拖进“应用程序”。
2. 这是未公证的极客向构建。首次运行若被 macOS 拦截，可在终端执行：

   ```sh
   xattr -dr com.apple.quarantine "/Applications/DSH Desktop.app"
   ```

3. 再正常打开应用。Node、npm 和首个可用 dsh 版本都已包含在 App 中，无需预装开发环境。

也可以先尝试打开一次，然后到“系统设置 → 隐私与安全性”选择“仍要打开”。不要全局关闭 Gatekeeper。

## 更新策略

- 启动只读取本机当前版本，不等待网络。
- 界面打开 60 秒后，若距离上次成功检查已满 24 小时，后台读取 npm `dist-tags.latest`。
- 新版本会安装到独立的版本目录，使用捆绑的 Node/npm 验证版本并完成一次启动冒烟测试。
- 正在运行的版本不会被替换；新版本在下次启动时切换。
- 若新版本启动失败，应用自动退回上一版本。网络或安装失败也不会影响当前版本。
- “DSH Desktop → 检查 dsh 更新…”可手动检查；“重新启动”可应用已就绪的更新。

Harness 的用户配置和会话仍由上游存放在 `~/.dsh`。外壳自己的版本缓存与日志位于 `~/Library/Application Support/com.iobee.dsh-desktop/`。

## 本机构建

要求：Apple Silicon Mac、Rust、Xcode Command Line Tools，以及用于执行准备脚本的 Node。

```sh
npm install
npm run desktop:build
```

`prepare:runtime` 固定下载 Node 24.19.0 LTS 的官方 arm64 macOS 发行包，校验 Node 官方 SHA-256，并捆绑 npm 11.19.0，用它安装当时 `@deepseek-ai/dsh@latest`。构建产物在 `src-tauri/target/release/bundle/`。

当前发行物没有 Developer ID 签名或 Apple 公证，适合小范围极客用户；以后若获得证书，外壳签名更新可以与 npm dsh 更新保持两条独立通道。

## 许可与来源

本外壳使用 MIT License。DeepSeek Harness 及其依赖保留各自的许可与版权；上游 npm 包被原样安装为运行时依赖。
