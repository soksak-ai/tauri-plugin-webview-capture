// Linux 캡처 — WebKitGTK(GTK3) WebViewExt::snapshot → cairo::Surface → PNG.
//
// 컴파일은 소비 앱의 3-OS CI 게이트(ubuntu-24.04 cargo check)로 검증된다 — 첫 실검사가
// 드러낸 정정: webkit2gtk 2.0.2(gtk-rs 0.18 세대)엔 prelude 모듈이 없고(WebViewExt 직접
// import) 메서드는 get_ 접두사 없는 snapshot 이며, write_to_png 은 cairo-rs 의 png
// feature 뒤에 있다. 런타임 캡처 동작은 Linux 실기 검증이 남아 있다. cairo-rs 버전은
// webkit2gtk 의 transitive cairo 와 같아야 try_from 이 타입 일치(0.18 핀).
use tauri::{Runtime, Webview};

pub(crate) async fn capture<R: Runtime>(win: &Webview<R>, path: &str) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(1);

    win.with_webview(move |pw| {
        use cairo::ImageSurface;
        // webkit2gtk 2.0.2(구세대 gtk-rs)엔 prelude 모듈이 없다 — 확장 트레이트를 직접 import 한다.
        // gio 는 재노출 안 되므로 직접 의존(Cargo.toml gio 0.18)에서 가져온다.
        use gio::Cancellable;
        use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

        // with_webview 클로저는 GTK 메인 스레드에서 실행된다(tauri 보장).
        // inner() = webkit2gtk::WebView(소유 클론). get_snapshot 은 비동기 — 콜백이
        // GLib 메인 컨텍스트에서 호출되며 결과를 sync_channel 로 보낸다(메인 스레드 안 막음).
        let webview = pw.inner();
        let tx = tx.clone();
        // webkit2gtk 2.0.2(gtk-rs 0.18 세대)는 get_ 접두사를 뗀 snapshot 이다(cargo check 실측).
        webview.snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::NONE,
            // None 을 넘기려면 타입 명시 필요(IsA<Cancellable> 추론 불가).
            None::<&Cancellable>,
            move |result| {
                let outcome = (|| -> Result<Vec<u8>, String> {
                    let surface = result.map_err(|e| format!("get_snapshot 실패: {e}"))?;
                    // cairo::Surface → ImageSurface(이미지가 아니면 원본을 Err 로 돌려줌).
                    let image = ImageSurface::try_from(surface)
                        .map_err(|_| "스냅샷이 이미지 서피스가 아님".to_string())?;
                    image.flush(); // 펜딩 드로잉 반영
                    let mut bytes: Vec<u8> = Vec::new();
                    image
                        .write_to_png(&mut bytes)
                        .map_err(|e| format!("PNG 인코딩 실패: {e}"))?;
                    Ok(bytes)
                })();
                let _ = tx.try_send(outcome);
            },
        );
    })
    .map_err(|e| e.to_string())?;

    let bytes = tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "snapshot 시간 초과".to_string())?
    })
    .await
    .map_err(|e| e.to_string())??;

    std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}

// 가림 감지: Linux(WebKitGTK)엔 macOS 식 occlusion 스로틀이 없다(최소화/숨김 때만) → no-op.
pub(crate) fn set_occlusion<R: Runtime>(_win: &Webview<R>, _enabled: bool) -> Result<(), String> {
    Ok(())
}

pub(crate) async fn arm_capture<R: Runtime>(_win: &Webview<R>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(200))
    })
    .await
    .map_err(|e| e.to_string())
}

pub(crate) fn disarm_capture<R: Runtime>(_win: &Webview<R>) {}

// snapshot_region — WebKitGTK 스냅샷의 rect crop 은 아직 미구현(파일 스냅샷만 지원).
pub(crate) async fn capture_region_png<R: Runtime>(
    _win: &Webview<R>,
    _rect: Option<(f64, f64, f64, f64)>,
) -> Result<Vec<u8>, String> {
    Err("snapshot_region: Linux 미구현".into())
}
