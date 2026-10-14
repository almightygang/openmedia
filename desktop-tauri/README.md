# Openmedia (Tauri)

Rust + Tauri v2 desktop anime app: React frontend, native webview shell, no
Chromium bundling. Targets: Windows x64 + Linux x64 only.

## Layout

- `src/` — React frontend.
  - `src/lib/tauri-api.ts` — installs the `window.*` globals
    (`theme`, `app`, `streamProvider`, `anilist`, …) on top of
    `invoke()`, so pages/hooks/components call one stable surface.
- `src-tauri/` — Rust backend (`#[tauri::command]` per feature area).
- `scripts/download-ffmpeg.mjs` — fetches pinned ffmpeg statics
  (`eugeneware/ffmpeg-static b4.4.1`, sha256-verified) into
  `src-tauri/binaries/` with Tauri sidecar naming. Binaries are gitignored.

## Dev

```bash
npm install
npm run download-ffmpeg   # linux + windows amd64 sidecars
npm run tauri:dev
```

## Build (amd64 only)

```bash
npm run build              # vite frontend -> dist/
npm run tauri:build        # deb, rpm, AppImage, nsis
```

## Status

- [x] Scaffold: window, deep-link (`openmedia://`), single-instance, updater stub
- [x] Commands: app/theme/window/external/privacy/history (working, file-backed store)
- [x] Bridge: `window.*` shim, `tsc` clean, `vite build` clean, `cargo` clean
- [x] Stream proxy: axum localhost server — Referer-injecting passthrough,
  HLS master filtering + opaque-URL rewrite, ASS→WebVTT, VOD playlist
  synthesis, ffmpeg sequential transcode jobs with seek-restart and 410
  past-EOF handling (`src-tauri/src/proxy/`)
- [x] AniList: public + authed GraphQL (media page/search, details, popular
  season w/ 1h cache, list CRUD, favourites, watch-progress sync w/
  incognito respect), system-browser OAuth + `openmedia://` deep-link
  callback + pin-page manual token (`src-tauri/src/anilist/`)
- [x] Provider: AnimeParadise only, live-verified end-to-end
  (search → details → episodes → stream bytes). Six other upstreams were
  removed after live verification — see root README for verdicts.
- [ ] Updater wiring (endpoint + pubkey placeholders in `tauri.conf.json`)
