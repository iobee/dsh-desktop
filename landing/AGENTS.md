# Prototype Instructions

Run the local server yourself and open the preview in the browser available to this environment. Do not give the user server-start instructions when you can run it.

Before making substantial visual changes, use the Product Design plugin's `get-context` skill when the visual source is unclear or no longer matches the current goal. When the user gives durable prototype-specific design feedback, preferences, or decisions, record them in `AGENTS.md`.

When implementing from a selected generated mock, treat that image as the source of truth for layout, component anatomy, density, spacing, color, typography, visible content, and hierarchy.

## Selected landing direction

- Use the user-selected dark, cinematic mock at `design-reference.png` as the visual source of truth.
- Preserve its premium black-and-cobalt presentation, large centered hero, oversized product window, three concise benefits, and preview caveat.
- Keep one download zone in the hero. Do not repeat platform pills or download buttons in the footer; the footer closes with compact navigation.
- Keep the product screenshot lossless and Retina-ready: provide an image candidate at least twice the maximum rendered width, and do not reintroduce a heavily compressed JPEG.
- Keep product claims grounded in the parent `dsh-desktop` repository. Do not copy the mock's incorrect DeepSeek copyright attribution.

The canonical public deployment is GitHub Pages at `https://iobee.github.io/dsh-desktop/`. Keep public asset URLs compatible with both the Pages subpath and local root previews through Vite's `BASE_URL`.

Build app UI in `src/`. Keep `.openai/hosting.json`, `worker/index.js`, `scripts/prepare-sites-build.mjs`, and `tests/sites-worker.test.mjs` intact so the same local prototype can be handed to Sites. Before a Sites handoff, run `npm run build` and `npm run test:sites`; the build must leave `dist/client/index.html`, `dist/server/index.js`, and `dist/.openai/hosting.json`.
