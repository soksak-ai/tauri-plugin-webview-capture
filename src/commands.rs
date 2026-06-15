use crate::error::{Error, Result};
use crate::platform;
use tauri::{Runtime, WebviewWindow};

// 부모 디렉터리 보장 — 명령이 자급자족(호출자 mkdir 불필요).
fn ensure_parent(path: &str) -> Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// 호출한 창의 webview 내용을 PNG 로 저장(MW2 — webview_window 자동 인지, 단일 "main" 가정 제거).
/// 다른 앱에 완전히 가려져 있어도 캡처된다(캡처 순간만 가림감지 자동 해제→복원, macOS). WebGL 포함.
#[tauri::command]
pub async fn snapshot<R: Runtime>(
    webview_window: WebviewWindow<R>,
    path: String,
) -> Result<String> {
    ensure_parent(&path)?;
    platform::arm_capture(&webview_window).await.map_err(Error::Capture)?;
    let r = platform::capture(&webview_window, &path).await;
    platform::disarm_capture(&webview_window); // 항상 복원
    r.map_err(Error::Capture)?;
    Ok(path)
}

/// 연사 캡처 → dir/f0000.png .. (내장 동영상 소스). 가려져 있어도 모든 프레임이
/// 렌더된다(연사 동안 가림감지 해제, macOS). 반환=찍은 프레임 수.
#[tauri::command]
pub async fn record<R: Runtime>(
    webview_window: WebviewWindow<R>,
    dir: String,
    frames: u32,
    interval_ms: u64,
) -> Result<u32> {
    use std::time::Duration;
    std::fs::create_dir_all(&dir)?; // 폴더 보장(1회)
    let n = frames.min(600); // 폭주 방지 상한
    platform::arm_capture(&webview_window).await.map_err(Error::Capture)?;
    let mut err: Option<String> = None;
    for i in 0..n {
        let path = format!("{dir}/f{i:04}.png");
        if let Err(e) = platform::capture(&webview_window, &path).await {
            err = Some(e);
            break;
        }
        if i + 1 < n {
            let _ = tauri::async_runtime::spawn_blocking(move || {
                std::thread::sleep(Duration::from_millis(interval_ms))
            })
            .await;
        }
    }
    platform::disarm_capture(&webview_window); // 항상 복원
    if let Some(e) = err {
        return Err(Error::Capture(e));
    }
    Ok(n)
}

/// 가림감지 토글(macOS). false 면 다른 앱에 완전히 가려져도 렌더를 멈추지 않는다
/// (상시 백그라운드 캡처용 — 배터리 비용 주의). Windows/Linux 엔 동등 스로틀이 없어
/// no-op. snapshot/record 는 캡처 순간만 자동으로 끄므로 평소엔 불필요.
#[tauri::command]
pub fn set_occlusion<R: Runtime>(webview_window: WebviewWindow<R>, enabled: bool) -> Result<()> {
    platform::set_occlusion(&webview_window, enabled).map_err(Error::Capture)
}
