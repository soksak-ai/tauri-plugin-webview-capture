// macOS 캡처 — WKWebView.takeSnapshotWithConfiguration → NSImage → PNG.
// 가림감지 토글은 _setWindowOcclusionDetectionEnabled(WKWebView 사적 API): 끄면 창이
// 완전히 덮여도 렌더를 유지해 캡처 가능. 비 App Store 앱이라 허용.
use tauri::{AppHandle, Manager, Runtime};

// 가림 감지 토글(동기). enabled=false 면 덮여도 렌더 유지 → 캡처 가능.
pub(crate) fn set_occlusion<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> Result<(), String> {
    let win = app.get_webview_window("main").ok_or("main webview 없음")?;
    win.with_webview(move |pw| unsafe {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        let wk = pw.inner() as *mut AnyObject;
        let _: () = msg_send![&*wk, _setWindowOcclusionDetectionEnabled: enabled];
    })
    .map_err(|e| e.to_string())
}

// 캡처 직전: 가림감지를 끄고 렌더 재개 여유(200ms). 끝나면 disarm 으로 복원.
pub(crate) async fn arm_capture<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    set_occlusion(app, false)?;
    tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(200))
    })
    .await
    .map_err(|e| e.to_string())
}

// 캡처 후: 가림감지 복원(실패는 무시 — 캡처 결과를 가리지 않는다).
pub(crate) fn disarm_capture<R: Runtime>(app: &AppHandle<R>) {
    let _ = set_occlusion(app, true);
}

// 단일 네이티브 캡처 → path. 완료 핸들러(블록)가 sync_channel 로 결과를 보내고
// 본체는 spawn_blocking 에서 기다린다 — with_webview 클로저(메인 스레드)를 막지 않는다.
pub(crate) async fn capture<R: Runtime>(app: &AppHandle<R>, path: &str) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let win = app.get_webview_window("main").ok_or("main webview 없음")?;
    let (tx, rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let out = path.to_string();

    win.with_webview(move |pw| {
        use block2::RcBlock;
        use objc2::msg_send;
        use objc2::rc::Retained;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
        use objc2_foundation::{NSData, NSError, NSString};
        use objc2_web_kit::WKWebView;

        unsafe {
            let wk = &*(pw.inner() as *const WKWebView);
            let tx = tx.clone();
            let out = out.clone();
            // 완료 핸들러: NSImage → TIFF → NSBitmapImageRep → PNG → 파일.
            let block = RcBlock::new(move |img: *mut NSImage, error: *mut NSError| {
                let outcome = (|| -> Result<(), String> {
                    if !error.is_null() {
                        return Err((*error).localizedDescription().to_string());
                    }
                    if img.is_null() {
                        return Err("snapshot 이미지 nil".into());
                    }
                    let image = &*img;
                    let tiff: Option<Retained<NSData>> = msg_send![image, TIFFRepresentation];
                    let tiff = tiff.ok_or("TIFF 표현 실패")?;
                    let rep: Option<Retained<NSBitmapImageRep>> =
                        msg_send![objc2::class!(NSBitmapImageRep), imageRepWithData: &*tiff];
                    let rep = rep.ok_or("bitmap rep 실패")?;
                    // properties=nil → 기본 PNG 인코딩.
                    let png: Option<Retained<NSData>> = msg_send![
                        &*rep,
                        representationUsingType: NSBitmapImageFileType::PNG,
                        properties: std::ptr::null::<objc2::runtime::AnyObject>()
                    ];
                    let png = png.ok_or("PNG 인코딩 실패")?;
                    let nspath = NSString::from_str(&out);
                    let ok: bool = msg_send![&*png, writeToFile: &*nspath, atomically: true];
                    if ok {
                        Ok(())
                    } else {
                        Err("파일 쓰기 실패".into())
                    }
                })();
                let _ = tx.try_send(outcome);
            });
            // config=nil → 현재 보이는 전체 콘텐츠.
            let _: () = msg_send![
                wk,
                takeSnapshotWithConfiguration: std::ptr::null::<objc2::runtime::AnyObject>(),
                completionHandler: &*block
            ];
        }
    })
    .map_err(|e| e.to_string())?;

    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "snapshot 시간 초과".to_string())?
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}
