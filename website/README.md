# OpenWiki Website

This folder contains the static product website for OpenWiki.

## Live Site

- Production URL: https://openwiki.pages.dev
- Cloudflare Pages project: `openwiki`
- Hosting method: direct upload with Wrangler
- Custom domain: none

This deployment is isolated from other Cloudflare sites. Do not bind a custom
domain or change DNS records unless the user explicitly asks.

## Files

- `index.html` - the full static landing page.
- `assets/openwiki-icon.png` - app icon copied from `src-tauri/icons/icon.png`.
- `assets/screenshots/` - screenshots copied from `docs/screenshots/` so the
  Cloudflare upload is self-contained.

## Download Buttons

The Mac and Windows download buttons are updated in the browser at runtime.
`index.html` fetches:

```text
https://api.github.com/repos/kdsz001/OpenWiki/releases/latest
```

Then it selects:

- Mac: `OpenWiki_*_aarch64.dmg`
- Windows: `OpenWiki_*_x64-setup.exe`

If the release asset naming changes, update the regexes near the bottom of
`index.html`.

## Deploy

From the repository root:

```bash
npx wrangler pages deploy website --project-name=openwiki --branch=main --commit-dirty=true --commit-message="Deploy OpenWiki website"
```

The first deployment created the `openwiki` Pages project. Later deployments
reuse the same project.

## Verification

After deploying, check:

- https://openwiki.pages.dev loads.
- Logo and screenshots render.
- Mac download points to the latest `.dmg`.
- Windows download points to the latest `.exe`.
- No custom domain or DNS setting was changed.
