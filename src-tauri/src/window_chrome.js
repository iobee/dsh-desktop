(() => {
  const allowedOrigin =
    window.location.protocol === "tauri:" ||
    (window.location.protocol === "http:" && window.location.hostname === "127.0.0.1");
  if (!allowedOrigin) return;

  const trafficLightHoverWidth = 80;
  const trafficLightHoverHeight = 36;

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

    const hoverRegion = document.createElement("div");
    hoverRegion.setAttribute("data-dsh-traffic-light-hover-region", "");
    hoverRegion.setAttribute("aria-hidden", "true");
    Object.assign(hoverRegion.style, {
      position: "fixed",
      top: "0",
      left: "0",
      width: `${trafficLightHoverWidth}px`,
      height: `${trafficLightHoverHeight}px`,
      zIndex: "2147483647"
    });
    document.documentElement.appendChild(hoverRegion);

    let trafficLightsVisible = false;
    let visibilityUpdates = Promise.resolve();
    const setTrafficLightsVisible = (visible) => {
      if (visible === trafficLightsVisible) return;
      trafficLightsVisible = visible;
      hoverRegion.style.pointerEvents = visible ? "none" : "auto";
      visibilityUpdates = visibilityUpdates
        .then(() => window.__TAURI__.core.invoke("set_traffic_lights_visible", { visible }))
        .catch(() => {
          if (trafficLightsVisible === visible) {
            trafficLightsVisible = false;
            hoverRegion.style.pointerEvents = "auto";
          }
        });
    };

    hoverRegion.addEventListener("pointerenter", () => setTrafficLightsVisible(true));
    document.addEventListener(
      "pointermove",
      (event) => {
        if (
          trafficLightsVisible &&
          (event.clientX > trafficLightHoverWidth || event.clientY > trafficLightHoverHeight)
        ) {
          setTrafficLightsVisible(false);
        }
      },
      { capture: true, passive: true }
    );
    window.addEventListener("blur", () => setTrafficLightsVisible(false));
  };

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", installDragRegion, { once: true });
  } else {
    installDragRegion();
  }
})();
