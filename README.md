<img src="./desktop-tauri/src-tauri/icons/icon.png" width="128"/>

# OpenMedia

A fast, lightweight desktop anime app: React frontend on a Rust + Tauri v2
shell. No Electron, no bundled Chromium — the system webview plus a native
backend (local HLS proxy with ffmpeg transcode, AniList sync, provider
requests in Rust).

Fork of [wuon/openanime](https://github.com/wuon/openanime), rebuilt 1:1 on
Tauri. The current provider implementation builds on ideas from
[pystardust/ani-cli](https://github.com/pystardust/ani-cli), while keeping
the app ready for additional providers over time.

## Status

Working: search, recent uploads, show details, episode listing, HLS watch
playback (direct + ffmpeg-transcoded), watch history, AniList browsing/lists/
favourites/watch-progress sync, themes, custom titlebars.

Provider: **AnimeParadise only** (sub-only). Six other upstreams were removed
after live verification showed none of them resolves a playable stream in
this build:

| Provider | Verdict |
|---|---|
| AnimeParadise | Works (search → details → episodes → stream bytes verified live) |
| AllAnime | Streams dead — site removed the `__aaCrypto` handshake the `aaReq` crypto needs (`AA_CRYPTO_MISSING`) |
| HiAnime | Search recovered, but the Zoko embed CDN is unreachable → no streams |
| ReAnime | Catalog works; flixcloud embed decrypt needs the site WASM module |
| Senshi | API host TLS-unreachable from here |
| AnimePahe | Cloudflare-403 on search |
| AniDB | `anidb.app` now redirects to `anilab.so` (rewritten SPA, legacy markup gone) |

Targets: Windows x64 + Linux x64.

## Installation

Download the latest release for your platform (`AppImage` / `.deb` / `.rpm`
for Linux, `nsis` `.exe` for Windows) and run it. Linux note: the AppImage
needs `libwebkit2gtk-4.1` and `libgtk-3` present (stock on Ubuntu 22.04+).

No auto-update yet — check releases manually for now.

### AniList (optional)

Settings → connect AniList to sync your list, favourites, and watch
progress. Two ways to sign in:

- **Pin page (easiest):** open the pin page, paste the token. No setup needed.
- **Browser OAuth:** requires your own AniList API client id — register an
  app at `anilist.co/settings/developer` with redirect URI
  `openmedia://auth/anilist/callback`, then set `ANILIST_CLIENT_ID` in
  `desktop-tauri/src-tauri/src/anilist/oauth.rs` and rebuild.

## Build from source

Requirements: Node 20+, Rust stable, and on Linux the WebKitGTK dev
libraries (`libwebkit2gtk-4.1-dev libgtk-3-dev` plus
`libayatana-appindicator3-dev librsvg2-dev patchelf` for bundling).

```bash
cd desktop-tauri
npm install
npm run download-ffmpeg   # pinned ffmpeg sidecars (sha256-verified)
npm run tauri:dev          # dev window with hot reload
npm run build              # vite frontend -> dist/
npm run tauri:build        # deb, rpm, AppImage, nsis
```

Tests: `cargo test` in `desktop-tauri/` (unit + live API checks),
`npx tsc --noEmit` for the frontend.

## Layout

- `desktop-tauri/` — the app (frontend + Rust backend, see its README).
- `LICENSE.txt` — license.

## License

[GPL-3.0](LICENSE.txt)
