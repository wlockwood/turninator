# Deploying the web app to GitHub Pages

This guide publishes the browser version of Turninator (the Vite + Svelte app in [`web/`](../web/)) as a static GitHub Pages site,
built and deployed by GitHub Actions on every push.

The result is a project site at:

```
https://<owner>.github.io/<repo>/
```

for example `https://wlockwood.github.io/turninator/`.

## Why this works without extra configuration

The web app already fits GitHub Pages:

- **Relative base path.** [`web/vite.config.ts`](../web/vite.config.ts) sets `base: './'`, so the build works under any path,
  including the `/<repo>/` prefix of a project site or the root of a custom domain. The service worker and the OIDN denoiser weights
  are loaded through `import.meta.env.BASE_URL`, so they follow the same prefix.
- **Static output only.** `npm run build` writes a plain static site to `web/dist/` (about 3 MB, including the 640 KB OIDN weights).
  Nothing runs on a server.
- **HTTPS.** WebGPU, service workers, OPFS and the File System Access API need a secure context. GitHub Pages always serves
  `*.github.io` over HTTPS.
- **No Jekyll processing.** A site deployed by an Actions workflow is uploaded as-is, so no `.nojekyll` file is needed.

### Cross-origin isolation headers

GitHub Pages cannot set custom HTTP headers. The Vite dev and preview servers send
`Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`. On Pages those headers are missing, so the page is **not**
cross-origin isolated and `SharedArrayBuffer` is unavailable.

Nothing in `web/src` uses `SharedArrayBuffer` or checks `crossOriginIsolated` today; the headers exist for future threaded WASM.
If that changes, the Pages deployment will need either a header-injecting service worker
(the [coi-serviceworker](https://github.com/gzuidhof/coi-serviceworker) pattern, merged into `web/public/sw.js`) or a host that supports
custom headers (Cloudflare Pages, Netlify).

## Prerequisites

1. **A GitHub repository for this project.** The local repository has no remote yet. Create an empty repository on GitHub (no README,
   no license, so nothing conflicts), then:

   ```sh
   git remote add origin https://github.com/<owner>/<repo>.git
   git push -u origin development
   ```

   Pages is free for public repositories. A private repository needs GitHub Pro, Team or Enterprise, and the published site is still
   public unless you use Enterprise Cloud's private Pages.

2. **`web/` committed.** `web/` is currently untracked. Commit it before deploying. Its own [`.gitignore`](../web/.gitignore) already excludes
   `node_modules/`, `dist/` and `test-results/`, so only sources, `package-lock.json` and `public/` (including `public/oidn/` and its
   `LICENSE.txt`) are committed.

3. **A deploy branch.** Pick the branch whose pushes publish the site. The workflow below uses `main`. This repository works on
   `development` and has no local `main` branch, so either create `main` or change the branch name in the workflow (see
   [Deploying from a branch other than the default](#deploying-from-a-branch-other-than-the-default)).

## Step 1: Add the workflow

Create `.github/workflows/pages.yml` at the repository root (not inside `web/`):

```yaml
name: Deploy web app to GitHub Pages

on:
  push:
    branches: [main]
    paths:
      - 'web/**'
      - '.github/workflows/pages.yml'
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

# One deployment at a time; let an in-progress deployment finish.
concurrency:
  group: pages
  cancel-in-progress: false

jobs:
  build:
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: web
    steps:
      - uses: actions/checkout@v5

      - uses: actions/setup-node@v5
        with:
          node-version: 22
          cache: npm
          cache-dependency-path: web/package-lock.json

      - run: npm ci
      - run: npm run check
      - run: npm test
      - run: npm run build

      - uses: actions/configure-pages@v5

      - uses: actions/upload-pages-artifact@v4
        with:
          path: web/dist

  deploy:
    needs: build
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - id: deployment
        uses: actions/deploy-pages@v4
```

How the workflow behaves:

- **Triggers.** It runs on pushes to `main` that touch `web/` or the workflow itself, so commits to the native Rust app don't
  redeploy the site. `workflow_dispatch` adds a **Run workflow** button on the Actions tab for manual deploys.
- **Node 22.** Vite 8 needs Node `^20.19.0 || >=22.12.0`.
- **Checks before publishing.** `npm run check` (svelte-check) and `npm test` (Vitest) must pass before anything is built. The
  PrusaSlicer G-code fixture tests skip themselves when `web/test-results/gcode/` is absent, which it always is in CI, so no
  fixtures are needed.
- **Lockfile.** `npm ci` installs exactly what `web/package-lock.json` records. The lockfile was generated on Windows but already includes
  the Linux native bindings (`@rolldown/binding-linux-x64-gnu`, `lightningcss-linux-x64-gnu`) the Ubuntu runner needs.
- **Playwright** is a dev dependency, but `npm ci` doesn't download browsers, and the workflow doesn't run the Playwright tools in
  `web/tools/` (they need a real GPU and a dev server on port 5199).

The action major versions above were current when this guide was written. Newer majors are fine to adopt; check each action's
release notes for breaking changes.

## Step 2: Enable Pages

1. On GitHub, open the repository's **Settings → Pages**.
2. Under **Build and deployment → Source**, choose **GitHub Actions**. Leave **Deploy from a branch** unselected; this workflow
   doesn't use a `gh-pages` branch.

This step is required. Without it, `actions/deploy-pages` fails because Pages isn't enabled for the repository.

## Step 3: Deploy

Push the workflow and `web/` to the deploy branch:

```sh
git add .github/workflows/pages.yml web
git commit -m "ci(web): deploy the web app to GitHub Pages"
git push origin main
```

Then open the **Actions** tab, select **Deploy web app to GitHub Pages**, and wait for both jobs to finish. The `deploy` job's summary
links to the live URL, which is also shown under **Settings → Pages**.

To redeploy without a code change (for example, after a failed run), use **Actions → Deploy web app to GitHub Pages → Run workflow**.

## Step 4: Verify the deployment

Open the site in Edge or Chrome (WebGPU is needed for path tracing), then:

1. **The app loads.** DevTools Console shows no 404s. Assets should come from `/<repo>/assets/…`, not `/assets/…`.
2. **A model opens.** Drop an STL from [`test-models/`](../test-models/) onto the page.
3. **The service worker registered.** DevTools → Application → Service workers lists `sw.js` with scope `/<repo>/`.
4. **The denoiser weights load.** Render a path-traced still; DevTools → Network shows `oidn/rt_hdr_calb_cnrm_small.tza` with status 200.
5. **Offline works.** After one full load, tick **Offline** in DevTools → Network and reload. The app shell should still come up.
6. **Install works (optional).** The browser's install button installs the PWA, and `.stl`/`.gcode` file handling registers.

### Testing the production build locally first

`npm run preview` serves `web/dist/` at the site root and still sends the COOP/COEP headers, so it doesn't fully reproduce Pages. To
mimic a project site (sub-path, no special headers), serve the build from a parent folder:

```sh
cd web
npm run build
mkdir -p ../.pages-test && rm -rf ../.pages-test/turninator && cp -r dist ../.pages-test/turninator
python -m http.server 8080 --directory ../.pages-test
```

Then open `http://localhost:8080/turninator/`. `localhost` counts as a secure context, so WebGPU and the service worker behave as on
Pages. Delete `.pages-test/` afterwards.

## Updates and caching

Each push to the deploy branch that touches `web/` publishes a new build. What returning visitors see:

- **App shell** (`index.html`, the manifest, `sw.js`): the service worker fetches these network-first, so a new deploy shows up on the
  next load. GitHub Pages sends `Cache-Control: max-age=600`, so allow up to about 10 minutes.
- **Hashed bundles** (`assets/*-<hash>.js`): served cache-first, which is safe because every build changes the file names.
- **OIDN weights** (`oidn/*.tza`): also served cache-first, but their file names aren't hashed. If you replace a weights file
  under the same name, bump `CACHE` in [`web/public/sw.js`](../web/public/sw.js) (`'turninator-v1'` → `'turninator-v2'`). The new
  service worker then deletes the old cache on activation. Otherwise returning visitors keep the old weights.

## Optional: custom domain

1. In **Settings → Pages → Custom domain**, enter the domain and save. With Actions deployments, the domain lives in settings,
   so no `CNAME` file is needed in the build.
2. Create the DNS records GitHub shows: a `CNAME` to `<owner>.github.io` for a subdomain, or `A`/`AAAA` records for an apex domain.
3. Once the certificate is issued, tick **Enforce HTTPS**. The app needs HTTPS for WebGPU and the service worker.

The relative base path means no build change is needed when moving from `/<repo>/` to a domain root. A previously installed PWA
is tied to its origin, though, so visitors must reinstall it from the new domain.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `deploy` job fails with `Get Pages site failed` or `HttpError: Not Found` | Pages isn't enabled, or its source is **Deploy from a branch**. Set the source to **GitHub Actions** (Step 2). |
| `deploy` job fails with `Branch "<name>" is not allowed to deploy to github-pages due to environment protection rules` | The workflow ran from a branch that the `github-pages` environment doesn't allow. See the next section. |
| Workflow didn't run after a push | The push didn't touch `web/**` or the workflow file, or it went to a different branch. Use **Run workflow** to deploy manually. |
| `npm ci` fails with `Missing: … from lock file` | `package.json` and `package-lock.json` disagree. Run `npm install` in `web/` and commit the updated lockfile. |
| `npm ci` fails with an unsupported-engine error, or Vite exits on startup | The Node version is too old for Vite 8. Keep `node-version: 22` (or ≥ 20.19). |
| Blank page; Console shows 404s for `/assets/…` without the repo name | `base` in `web/vite.config.ts` was changed from `'./'`. Restore it, or set it to `'/<repo>/'`. |
| Old version still showing after a deploy | Wait out the 10-minute cache, then reload. If it persists, DevTools → Application → Service workers → **Update**, or clear site data. |
| Path tracing unavailable | The browser has no WebGPU (for example Firefox without the flag, or blocklisted GPU drivers). The raster studio still works. This is a browser limitation, not a deployment problem. |

### Deploying from a branch other than the default

When Pages is enabled, GitHub creates a `github-pages` environment that, by default, accepts deployments only from the repository's
default branch. To publish from another branch (for example `development`):

1. Change `branches: [main]` in the workflow to that branch.
2. In **Settings → Environments → github-pages → Deployment branches and tags**, add a rule for the branch.

Alternatively, make that branch the repository's default branch in **Settings → General**.
