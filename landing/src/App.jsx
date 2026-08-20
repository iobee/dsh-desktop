import {
  AppleLogo,
  ArrowSquareOut,
  ArrowsClockwise,
  Cube,
  ShieldCheck,
  Warning,
} from "@phosphor-icons/react";

const links = {
  github: "https://github.com/iobee/dsh-desktop",
  docs: "https://github.com/iobee/dsh-desktop#使用方式",
  releases: "https://github.com/iobee/dsh-desktop/releases",
  mac: "https://github.com/iobee/dsh-desktop/releases/download/v0.1.9/DSH.Desktop_0.1.9_aarch64.dmg",
};

const assets = {
  icon: `${import.meta.env.BASE_URL}assets/dsh-desktop-icon.png`,
  screenshot: `${import.meta.env.BASE_URL}assets/dsh-desktop-dark.png`,
  screenshotRetina: `${import.meta.env.BASE_URL}assets/dsh-desktop-dark@2x.png`,
};

const benefits = [
  {
    title: "极简轻量",
    description: "内置必要运行环境，下载即可打开；桌面壳只负责启动、更新和可选终端入口。",
    Icon: Cube,
  },
  {
    title: "上游原样",
    description: "不 Fork、不修改 DSH 代码；直接运行上游发布的 @deepseek-ai/dsh npm 包。",
    Icon: ShieldCheck,
  },
  {
    title: "紧跟发版",
    description: "默认跟随 latest，也可加入 Beta 跟随 next；随时手动检查，并每 12 小时自动检查。",
    Icon: ArrowsClockwise,
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

function DownloadButton({ href, children, primary = false }) {
  return (
    <ExternalLink
      className={`button ${primary ? "button-primary" : "button-secondary"}`}
      href={href}
      label="下载 DSH Desktop macOS Apple Silicon 版"
    >
      <AppleLogo aria-hidden="true" size={22} weight="fill" />
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
            极简、轻量，让 DSH 成为随手可用的桌面应用。
            <br />
            不 Fork、不修改 DSH 代码，跟随 npm 发版，及时用上最新版。
          </p>

          <div className="hero-actions" aria-label="下载 DSH Desktop">
            <DownloadButton href={links.mac} primary>
              下载 macOS 版
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
              srcSet={`${assets.screenshot} 1566w, ${assets.screenshotRetina} 3132w`}
              sizes="(max-width: 520px) calc(100vw - 1.5rem), (max-width: 920px) calc(100vw - 2.5rem), (max-width: 1232px) calc(100vw - 9rem), 1088px"
              alt="DSH Desktop 暗色主题主界面，展示工作区、模式、权限与模型选择入口"
              decoding="async"
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
              面向技术用户的预览构建。当前版本尚未进行 Apple Notarization。
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
              <p>极简轻量、紧跟上游的 DeepSeek Harness 桌面入口。</p>
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
