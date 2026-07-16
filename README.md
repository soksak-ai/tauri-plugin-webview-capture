# tauri-plugin-webview-capture

A Tauri 2 plugin that captures a webview's rendered output (DOM + WebGL
composite) as PNG. It captures the **webview's own render**, not the screen —
so on macOS a window fully occluded by other apps is still captured without
bringing it to the front.

Tauri's core has no webview capture API. This plugin captures through each
OS's native API:

| OS | API | Status |
|----|-----|--------|
| macOS | ScreenCaptureKit `SCScreenshotManager` (own-process windows) | runtime-verified, including occluded capture |
| Windows | `ICoreWebView2.CapturePreview` | compiles in the consuming app's three-OS CI gate; runtime capture unverified |
| Linux | WebKitGTK `WebView::snapshot` (GTK3) | compiles in the consuming app's three-OS CI gate; runtime capture unverified |

On macOS it captures the **OS compositor's window composite** rather than a
single webview's `takeSnapshot` — the main webview and every child webview
(embedded browser views included) land in one image with no holes.
`getCurrentProcessShareableContent` scopes the capture to the app's own
windows, so no Screen Recording permission is needed. (The older
`CGWindowListCreateImage` is obsolete on macOS 15 and blocks during rendering,
so it was replaced with ScreenCaptureKit.)

## Commands

- `snapshot({ path })` — save a single PNG. Parent directories are created.
  Returns the saved path.
- `snapshot_region({ x?, y?, w?, h? })` — crop the window composite to a
  logical rect (CSS px, window coordinates) and return it as base64 PNG with
  no disk round trip. Omitting the rect captures the whole window. The crop
  uses `CGImageCreateWithImageInRect`, so only the region is encoded. macOS
  only (Windows/Linux return an error).
- `record({ dir, frames, intervalMs })` — save a burst of PNGs
  (`dir/f0000.png` …). A built-in video source.
- `set_occlusion({ enabled })` — toggle occlusion detection (macOS). `false`
  keeps the webview rendering in the background at a battery cost. Windows
  and Linux have no equivalent throttle, so it is a no-op there. `snapshot`
  and `record` disable it automatically for the capture instant.
- `analyze_regions({ dir, regions })` — mean luma of each region (fractional
  coordinates 0..1) per captured frame. Returns frames×regions.
- `analyze_frame_diffs({ dir, regions, thresh? })` — the fraction of pixels
  changed from the previous frame (change detection — it also catches
  same-brightness content transitions). Returns frames×regions.

## How occluded capture works

macOS WebKit throttles WebGL rendering in fully covered windows. `snapshot`
and `record` disable occlusion detection just before capturing
(`_setWindowOcclusionDetectionEnabled:false`, a private WKWebView API), give
rendering 200 ms to resume, capture, and restore the setting — the idle
battery cost stays zero.

Windows (WebView2) and Linux (WebKitGTK) have no macOS-style occlusion
throttle — they pause only when minimized or hidden and keep rendering while
covered — so `set_occlusion` being a no-op there is the correct behavior.

## Usage

```rust
// app entry point
tauri::Builder::default()
    .plugin(tauri_plugin_webview_capture::init())
    // ...
```

```json
// capabilities/*.json
"permissions": ["webview-capture:default"]
```

```ts
import { invoke } from "@tauri-apps/api/core";
await invoke("plugin:webview-capture|snapshot", { path: "/tmp/shot.png" });
```

## Status

The macOS implementation is runtime-verified, including occluded capture. The
Windows and Linux paths compile in the consuming app's three-OS `cargo check`
CI gate — that first real check surfaced and fixed genuine API drift (the
WebView2 completion-handler signature, the WebKitGTK 2.0 method names, cairo's
`png` feature) — and their runtime capture behavior still needs on-OS
verification.

---

한국어 안내는 [README.ko.md](README.ko.md)에 있습니다.

## License

MIT
