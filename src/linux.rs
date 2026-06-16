// Linux 캡처 — WebKitGTK(GTK3) WebViewExt::get_snapshot → cairo::Surface → PNG.
//
// [미검증] 이 머신(macOS)에선 컴파일되지 않는다. Linux CI/실기 검증 필요.
// 가장 가능성 높은 실패점: 메서드명 get_snapshot(신세대 gtk-rs 는 snapshot) —
// webkit2gtk 2.0.2(wry 가 쓰는 세대)에선 get_snapshot 으로 조사됨. cairo-rs 버전이
// webkit2gtk 의 transitive cairo 와 같아야 try_from 이 타입 일치(0.18 핀).
use tauri::{Runtime, Webview};

pub(crate) async fn capture<R: Runtime>(win: &Webview<R>, path: &str) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(1);

    win.with_webview(move |pw| {
        use cairo::ImageSurface;
        use webkit2gtk::gio::Cancellable;
        use webkit2gtk::prelude::WebViewExt;
        use webkit2gtk::{SnapshotOptions, SnapshotRegion};

        // with_webview 클로저는 GTK 메인 스레드에서 실행된다(tauri 보장).
        // inner() = webkit2gtk::WebView(소유 클론). get_snapshot 은 비동기 — 콜백이
        // GLib 메인 컨텍스트에서 호출되며 결과를 sync_channel 로 보낸다(메인 스레드 안 막음).
        let webview = pw.inner();
        let tx = tx.clone();
        webview.get_snapshot(
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
