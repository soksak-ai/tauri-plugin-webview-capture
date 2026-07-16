# tauri-plugin-webview-capture

webview 의 렌더 결과(DOM + WebGL 합성)를 PNG 로 캡처하는 Tauri 2 플러그인입니다.
화면 캡처가 아니라 **webview 자체의 렌더**를 잡기 때문에, macOS 에서는 창이 다른
앱에 완전히 가려져 있어도 전면 전환 없이 캡처됩니다.

Tauri 코어에는 webview 캡처 API 가 없습니다. 이 플러그인은 각 OS 의 네이티브
API 로 캡처합니다.

| OS | API | 상태 |
|----|-----|------|
| macOS | ScreenCaptureKit `SCScreenshotManager`(자기 프로세스 창) | 런타임 검증 완료(가림 캡처 포함) |
| Windows | `ICoreWebView2.CapturePreview` | 소비 앱의 3-OS CI 게이트에서 컴파일 검증; 런타임 캡처는 미검증 |
| Linux | WebKitGTK `WebView::snapshot`(GTK3) | 소비 앱의 3-OS CI 게이트에서 컴파일 검증; 런타임 캡처는 미검증 |

macOS 에서는 단일 webview 의 `takeSnapshot` 대신 **OS 컴포지터의 창 합성 결과**를
잡습니다 — 메인 webview 와 모든 child webview(내장 브라우저 뷰 포함)가 구멍 없이
한 장에 들어갑니다. `getCurrentProcessShareableContent` 로 자기 앱의 창만 잡으므로
화면 기록 권한이 필요하지 않습니다. (이전의 `CGWindowListCreateImage` 는 macOS 15
에서 폐기 예정이며 렌더 중 블로킹되어 ScreenCaptureKit 으로 교체했습니다.)

## 명령

- `snapshot({ path })` — 단일 PNG 를 저장합니다. 상위 폴더는 자동 생성되며,
  저장 경로를 반환합니다.
- `snapshot_region({ x?, y?, w?, h? })` — 창 합성 이미지를 논리 좌표(CSS px,
  창 기준) 사각형으로 잘라 base64 PNG 로 반환합니다(디스크를 거치지 않음).
  사각형을 생략하면 창 전체를 캡처합니다. 자르기는
  `CGImageCreateWithImageInRect` 를 사용해 해당 영역만 인코딩합니다. macOS
  전용입니다(Windows/Linux 는 오류를 반환).
- `record({ dir, frames, intervalMs })` — 연속 PNG(`dir/f0000.png` …)를
  저장합니다. 내장 동영상 소스입니다.
- `set_occlusion({ enabled })` — 가림 감지를 토글합니다(macOS). `false` 는
  백그라운드에서도 계속 렌더하게 하며 배터리 비용이 있습니다. Windows 와
  Linux 에는 동등한 스로틀이 없어 no-op 입니다. `snapshot`/`record` 는 캡처
  순간에만 자동으로 끕니다.
- `analyze_regions({ dir, regions })` — 캡처한 프레임마다 각 영역(0..1 분수
  좌표)의 평균 명도를 계산합니다. frames×regions 를 반환합니다.
- `analyze_frame_diffs({ dir, regions, thresh? })` — 직전 프레임과 달라진
  픽셀 비율을 계산합니다(변화 감지 — 같은 명도의 콘텐츠 전환도 잡습니다).
  frames×regions 를 반환합니다.

## 가림 캡처의 원리

macOS WebKit 은 완전히 덮인 창의 WebGL 렌더를 스로틀합니다. `snapshot` 과
`record` 는 캡처 직전 가림 감지를 끄고
(`_setWindowOcclusionDetectionEnabled:false`, WKWebView 비공개 API), 렌더가
재개될 여유 200ms 를 준 뒤 캡처하고, 끝나면 설정을 복원합니다 — 평상시 배터리
비용은 0 입니다.

Windows(WebView2)와 Linux(WebKitGTK)에는 macOS 식 가림 스로틀이 없습니다 —
최소화·숨김일 때만 멈추고 가려진 동안에는 계속 렌더합니다 — 따라서
`set_occlusion` 이 no-op 인 것이 올바른 동작입니다.

## 사용

```rust
// 앱 진입점
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

## 상태

macOS 구현은 가림 캡처를 포함해 런타임 검증이 끝났습니다. Windows 와 Linux
경로는 소비 앱의 3-OS `cargo check` CI 게이트에서 컴파일됩니다 — 첫 실검사가
실제 API 표류(WebView2 완료 핸들러 시그니처, WebKitGTK 2.0 메서드명, cairo 의
`png` feature)를 드러내 수정했습니다 — 런타임 캡처 동작은 해당 OS 에서의
검증이 남아 있습니다.

## License

MIT
