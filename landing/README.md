# DSH Desktop landing page

The public product page for DSH Desktop. GitHub Pages deploys it from `main` through `.github/workflows/pages.yml`.

```sh
npm ci
npm run dev
```

Use `npm run build:pages` for the static Pages artifact in `dist/client`. The workflow supplies `VITE_BASE_PATH=/dsh-desktop/`; local previews use `/` by default.

The project remains compatible with Sites. `npm run build` prepares both the static client and the Sites worker, and `npm run test:sites` checks that package.
