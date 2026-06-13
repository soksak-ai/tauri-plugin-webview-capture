# tauri-plugin-webview-capture

Tauri 2 플러그인 — webview 의 렌더 결과(DOM + WebGL 합성)를 PNG 로 캡처한다.
화면 캡처(`screencapture`)가 아니라 **webview 자체의 렌더**를 잡으므로, 창이 다른
앱에 **완전히 가려져 있어도(occluded)** 전면 전환 없이 캡처된다(macOS).

Tauri 코어엔 webview 캡처 API 가 없다. 이 플러그인은 `with_webview` escape hatch 로
각 OS 네이티브 API 를 호출한다:

| OS | API | 비고 |
|----|-----|------|
| macOS | `WKWebView.takeSnapshotWithConfiguration` | 검증 완료 |
| Windows | `ICoreWebView2.CapturePreview` | 미검증(CI 필요) |
| Linux | `webkit_web_view_get_snapshot` (WebKitGTK/GTK3) | 미검증(CI 필요) |

## 명령

- `snapshot({ path })` — 단일 PNG 저장. 부모 폴더 자동 생성. 반환=저장 경로.
- `record({ dir, frames, intervalMs })` — 연사 PNG(`dir/f0000.png`..) 저장. 내장 동영상 소스.
- `set_occlusion({ enabled })` — 가림감지 토글(macOS). `false`=상시 백그라운드 렌더(배터리 비용).
  Windows/Linux 엔 동등 스로틀이 없어 no-op. `snapshot`/`record` 는 캡처 순간만 자동으로 끈다.

## 가림 캡처 원리

macOS WebKit 은 완전히 덮인 창의 WebGL 렌더를 스로틀한다. `snapshot`/`record` 는 캡처
직전 `_setWindowOcclusionDetectionEnabled:false`(WKWebView 사적 API)로 가림감지를 끄고,
렌더 재개 여유(200ms) 후 캡처하고, 끝나면 복원한다 — 평소 배터리 비용은 0.

Windows(WebView2)·Linux(WebKitGTK)는 macOS 식 occlusion 스로틀이 없다(최소화/숨김
때만 멈춤, 가림 때는 계속 렌더) → `set_occlusion` 은 no-op 이 정답.

## 사용

```rust
// 앱 진입점
tauri::Builder::default()
    .plugin(tauri_plugin_webview_capture::init())
    // ...
```

```toml
# capabilities/*.json
"permissions": ["webview-capture:default"]
```

```ts
import { invoke } from "@tauri-apps/api/core";
await invoke("plugin:webview-capture|snapshot", { path: "/tmp/shot.png" });
```

## 상태

macOS 구현은 실기 검증(가림 캡처 포함) 완료. Windows/Linux 는 문서·소스 대조로
정확히 작성했으나 해당 OS CI/실기 빌드 검증이 필요하다(`src/windows.rs`, `src/linux.rs`
상단 주석의 caveat 참조).

## License

MIT
