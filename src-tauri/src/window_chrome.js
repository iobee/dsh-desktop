(() => {
  const allowedOrigin =
    window.location.protocol === "tauri:" ||
    (window.location.protocol === "http:" && window.location.hostname === "127.0.0.1");
  if (!allowedOrigin) return;

  const installDragRegion = () => {
    if (document.querySelector("[data-dsh-window-drag-region]")) return;

    const dragRegion = document.createElement("div");
    dragRegion.setAttribute("data-dsh-window-drag-region", "");
    dragRegion.setAttribute("data-tauri-drag-region", "");
    dragRegion.setAttribute("aria-hidden", "true");
    Object.assign(dragRegion.style, {
      position: "fixed",
      top: "0",
      right: "0",
      left: "72px",
      height: "24px",
      zIndex: "2147483647",
      userSelect: "none"
    });
    document.documentElement.appendChild(dragRegion);
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", installDragRegion, { once: true });
  } else {
    installDragRegion();
  }
})();
