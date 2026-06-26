//! tauri-plugin-webview-capture — webview 렌더(DOM + WebGL 합성)를 PNG 로 캡처.
//!
//! Tauri 코어엔 webview 캡처 API 가 없다. `with_webview` escape hatch 로 각 OS
//! 네이티브 API 를 직접 호출한다: macOS `WKWebView.takeSnapshotWithConfiguration`,
//! Windows `ICoreWebView2.CapturePreview`, Linux `webkit_web_view_get_snapshot`.
//!
//! 플랫폼 독립 오케스트레이션(가림감지 토글 래핑 · 디렉터리 보장 · 연사 루프)은
//! `commands` 에 한 번만 두고, 실제 네이티브 캡처/토글만 플랫폼 모듈(`platform`)이
//! 담당한다. 메인 webview(라벨 "main")를 stable API(`get_webview_window`)로 잡으므로
//! `unstable` 피처를 요구하지 않는다.

use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

mod commands;
mod error;

// 플랫폼별 네이티브 구현 — 셋 다 동일 인터페이스
// (capture / arm_capture / disarm_capture / set_occlusion)를 노출한다.
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

pub use error::{Error, Result};

/// 앱에서 `.plugin(tauri_plugin_webview_capture::init())` 로 등록.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("webview-capture")
        .invoke_handler(tauri::generate_handler![
            commands::snapshot,
            commands::record,
            commands::set_occlusion,
            commands::analyze_regions,
        ])
        .build()
}
