import {
  AppleLogo,
  ArrowSquareOut,
  ArrowsClockwise,
  Cube,
  ShieldCheck,
  Warning,
  WindowsLogo,
} from "@phosphor-icons/react";

const links = {
  github: "https://github.com/iobee/dsh-desktop",
  docs: "https://github.com/iobee/dsh-desktop#使用方式",
  releases: "https://github.com/iobee/dsh-desktop/releases",
  mac: "https://github.com/iobee/dsh-desktop/releases/download/v0.1.4/DSH.Desktop_0.1.4_aarch64.dmg",
  windows:
    "https://github.com/iobee/dsh-desktop/releases/download/v0.1.4/DSH.Desktop_0.1.4_x64-setup.exe",
};

const assets = {
  icon: `${import.meta.env.BASE_URL}assets/dsh-desktop-icon.png`,
  screenshot: `${import.meta.env.BASE_URL}assets/dsh-desktop-dark.jpeg`,
};

const benefits = [
  {
    title: "开箱即用",
    description: "内置 Node、npm 与可用 dsh 运行时，无需预装开发环境，下载即用。",
    Icon: Cube,
  },
  {
    title: "安静更新",
    description: "dsh 运行时与桌面外壳独立更新，后台检查不阻塞启动。",
    Icon: ArrowsClockwise,
  },
  {
    title: "失败可回退",
    description: "新运行时若启动失败，自动回退到上一个可用版本，确保稳定。",
    Icon: ShieldCheck,
  },
];

function ExternalLink({ href, className, children, label }) {
  return (
    <a
      aria-label={label}
      className={className}
      href={href}
      rel="noreferrer"
      target="_blank"
    >
      {children}
    </a>
  );
}

function DownloadButton({ href, platform, children, primary = false }) {
  const Icon = platform === "mac" ? AppleLogo : WindowsLogo;

  return (
    <ExternalLink
      className={`button ${primary ? "button-primary" : "button-secondary"}`}
      href={href}
      label={`下载 DSH Desktop ${platform === "mac" ? "macOS Apple Silicon" : "Windows x64"} 版`}
    >
      <Icon aria-hidden="true" size={22} weight="fill" />
      <span>{children}</span>
    </ExternalLink>
  );
}

export function App() {
  return (
    <div className="site-shell">
      <header className="topbar" aria-label="主导航">
        <ExternalLink className="brand" href={links.github} label="打开 DSH Desktop GitHub 仓库">
          <img src={assets.icon} alt="" />
          <span>DSH Desktop</span>
        </ExternalLink>

        <nav className="nav-links" aria-label="页面链接">
          <ExternalLink href={links.docs}>文档</ExternalLink>
          <ExternalLink href={links.releases}>更新日志</ExternalLink>
          <ExternalLink className="nav-github" href={links.github}>
            GitHub
            <ArrowSquareOut aria-hidden="true" size={15} />
          </ExternalLink>
        </nav>
      </header>

      <main>
        <section className="hero" aria-labelledby="hero-title">
          <p className="preview-badge">Developer Preview</p>
          <h1 id="hero-title">
            让 DeepSeek Harness
            <span>像普通应用一样打开</span>
          </h1>
          <p className="hero-copy">
            轻量的 macOS 与 Windows 桌面外壳。
            <br />
            内置 Node、npm 与可用 dsh 运行时，无需预装开发环境。
          </p>

          <div className="hero-actions" aria-label="下载 DSH Desktop">
            <DownloadButton href={links.mac} platform="mac" primary>
              下载 macOS 版
            </DownloadButton>
            <DownloadButton href={links.windows} platform="windows">
              下载 Windows 版
            </DownloadButton>
            <ExternalLink className="github-link" href={links.github}>
              查看 GitHub
              <ArrowSquareOut aria-hidden="true" size={16} />
            </ExternalLink>
          </div>

        </section>

        <section className="product-stage" aria-label="DSH Desktop 产品界面">
          <div className="product-frame">
            <img
              src={assets.screenshot}
              alt="DSH Desktop 暗色主题主界面，展示工作区、模式、权限与模型选择入口"
            />
          </div>
        </section>

        <section className="benefits" aria-label="产品特点">
          {benefits.map(({ title, description, Icon }) => (
            <article className="benefit" key={title}>
              <Icon aria-hidden="true" className="benefit-icon" size={31} weight="regular" />
              <div>
                <h2>{title}</h2>
                <p>{description}</p>
              </div>
            </article>
          ))}
        </section>

        <aside className="preview-notice" aria-label="开发者预览版说明">
          <Warning aria-hidden="true" size={43} weight="regular" />
          <div>
            <h2>开发者预览版</h2>
            <p>
              面向技术用户的预览构建。当前版本尚未进行 Apple Notarization 或 Windows Authenticode 签名。
            </p>
          </div>
        </aside>
      </main>

      <footer>
        <div className="footer-inner">
          <div className="footer-brand">
            <img src={assets.icon} alt="" />
            <div>
              <strong>DSH Desktop</strong>
              <p>独立、轻量、开箱即用的 DeepSeek Harness 桌面入口。</p>
            </div>
          </div>

          <nav className="footer-nav" aria-label="页脚链接">
            <ExternalLink href={links.docs}>文档</ExternalLink>
            <ExternalLink href={links.releases}>更新日志</ExternalLink>
            <ExternalLink className="footer-github" href={links.github}>
              GitHub
              <ArrowSquareOut aria-hidden="true" size={15} />
            </ExternalLink>
          </nav>
        </div>

        <p className="legal">
          © 2026 DSH Desktop · MIT License · 独立项目，运行内容来自 @deepseek-ai/dsh
        </p>
      </footer>
    </div>
  );
}
