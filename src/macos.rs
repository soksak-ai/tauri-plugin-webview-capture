// macOS 캡처 — CGWindowListCreateImage(창 전체). OS 컴포지터가 메인 webview + 모든 child
// webview(브라우저 뷰 등)를 합성한 창 이미지를 준다 → 콘텐츠 종류 무관 빠짐없이 캡처(메인 webview
// 만 잡던 takeSnapshot 의 hole 문제 해결). 가림감지 토글(_setWindowOcclusionDetectionEnabled,
// WKWebView 사적 API)로 완전히 덮여도 렌더 유지. 비 App Store 앱이라 허용.
use objc2::runtime::AnyObject;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use tauri::{Runtime, Webview};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCreateImage(
        bounds: NSRect,
        list_option: u32,
        window_id: u32,
        image_option: u32,
    ) -> *mut AnyObject; // CGImageRef(opaque)
    fn CGImageRelease(image: *mut AnyObject);
}

// CGWindowListOption / CGWindowImageOption 상수(CGWindow.h).
const INCLUDING_WINDOW: u32 = 1 << 3; // kCGWindowListOptionIncludingWindow
const IGNORE_FRAMING: u32 = 1 << 0; // kCGWindowImageBoundsIgnoreFraming
const BEST_RESOLUTION: u32 = 1 << 3; // kCGWindowImageBestResolution(retina)

// 가림 감지 토글(동기). enabled=false 면 덮여도 렌더 유지 → 캡처 가능. win = 대상 창(MW2 — 호출 창
// 자동 인지, 단일 "main" 가정 제거).
pub(crate) fn set_occlusion<R: Runtime>(win: &Webview<R>, enabled: bool) -> Result<(), String> {
    win.with_webview(move |pw| unsafe {
        use objc2::msg_send;
        let wk = pw.inner() as *mut AnyObject;
        let _: () = msg_send![&*wk, _setWindowOcclusionDetectionEnabled: enabled];
    })
    .map_err(|e| e.to_string())
}

// 캡처 직전: 가림감지를 끄고 렌더 재개 여유(200ms). 끝나면 disarm 으로 복원.
pub(crate) async fn arm_capture<R: Runtime>(win: &Webview<R>) -> Result<(), String> {
    set_occlusion(win, false)?;
    tauri::async_runtime::spawn_blocking(|| std::thread::sleep(std::time::Duration::from_millis(200)))
        .await
        .map_err(|e| e.to_string())
}

// 캡처 후: 가림감지 복원(실패는 무시 — 캡처 결과를 가리지 않는다).
pub(crate) fn disarm_capture<R: Runtime>(win: &Webview<R>) {
    let _ = set_occlusion(win, true);
}

// 창 전체(모든 child webview 합성)를 PNG 로 저장.
pub(crate) async fn capture<R: Runtime>(win: &Webview<R>, path: &str) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::sync_channel::<Result<(), String>>(1);
    let out = path.to_string();

    win.with_webview(move |pw| {
        let r = unsafe { capture_window(pw.inner() as *mut AnyObject, &out) };
        let _ = tx.try_send(r);
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

// 메인 스레드(with_webview 클로저)에서 동기 실행. wk = WKWebView 포인터.
unsafe fn capture_window(wk: *mut AnyObject, out: &str) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::{class, msg_send};
    use objc2_app_kit::NSBitmapImageFileType;
    use objc2_foundation::{NSData, NSString};

    // 호출 webview 의 NSWindow → windowNumber(CGWindowID).
    let nswindow: *mut AnyObject = msg_send![&*wk, window];
    if nswindow.is_null() {
        return Err("NSWindow 없음".into());
    }
    let num: isize = msg_send![&*nswindow, windowNumber];
    if num <= 0 {
        return Err("windowNumber 무효".into());
    }

    // bounds=CGRectNull({{INF,INF},{0,0}}) → 창 경계 자동. 창만 포함 + 프레임 무시 + 고해상도.
    let null_rect = NSRect::new(NSPoint::new(f64::INFINITY, f64::INFINITY), NSSize::new(0.0, 0.0));
    let cg = CGWindowListCreateImage(
        null_rect,
        INCLUDING_WINDOW,
        num as u32,
        IGNORE_FRAMING | BEST_RESOLUTION,
    );
    if cg.is_null() {
        return Err("CGWindowListCreateImage nil".into());
    }

    // CGImage → NSBitmapImageRep(initWithCGImage). alloc 은 class! + msg_send 로 직접(기존 패턴).
    let alloc: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
    let rep_raw: *mut AnyObject = msg_send![alloc, initWithCGImage: cg];
    CGImageRelease(cg); // initWithCGImage 가 자체 보관 → 원본 해제 안전
    let rep = Retained::from_raw(rep_raw).ok_or("NSBitmapImageRep nil")?;

    let png: Option<Retained<NSData>> = msg_send![
        &*rep,
        representationUsingType: NSBitmapImageFileType::PNG,
        properties: std::ptr::null::<AnyObject>()
    ];
    let png = png.ok_or("PNG 인코딩 실패")?;
    let nspath = NSString::from_str(out);
    let ok: bool = msg_send![&*png, writeToFile: &*nspath, atomically: true];
    if ok {
        Ok(())
    } else {
        Err("파일 쓰기 실패".into())
    }
}
