// macOS 캡처 — ScreenCaptureKit(SCScreenshotManager, macOS 14+). OS 컴포지터가 메인 webview + 모든 child
// webview(브라우저 뷰 등)를 합성한 창 이미지를 *논블로킹* 으로 준다 → 콘텐츠 종류 무관 빠짐없이 캡처(메인
// webview 만 잡던 takeSnapshot 의 hole 해결) + 렌더 중에도 멈추지 않는다. 구 CGWindowListCreateImage 는
// macOS 15 에서 obsolete + 블로킹/고부하라, 콘텐츠 전환의 무거운 렌더 중엔 WindowServer 와 경합해 호출당
// ~5s 로 늘어졌고(프레임 누적 hang) → SCK 컴포지터 경로로 교체. 가림감지 토글
// (_setWindowOcclusionDetectionEnabled, WKWebView 사적 API)로 완전히 덮여도 렌더 유지. 비 App Store 라 허용.
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSError, NSRect};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow,
};
use tauri::{Runtime, Webview};

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

// 캡처 출력 sink — 파일 저장 또는 (crop 후) PNG 바이트 반환. crop 은 물리 px(창 이미지 좌표).
enum Sink {
    File(String),
    Bytes { crop: Option<(f64, f64, f64, f64)> },
}

// 창 전체(모든 child webview 합성)를 PNG 로 저장. 메인스레드에서 대상 창 정보만 빠르게 뽑고(NSWindow
// 접근), 실제 캡처는 ScreenCaptureKit(off-main, 논블로킹)으로 한다 → 전환 렌더와 경합하지 않아 안 멈춘다.
pub(crate) async fn capture<R: Runtime>(win: &Webview<R>, path: &str) -> Result<(), String> {
    capture_sink(win, Sink::File(path.to_string())).await.map(|_| ())
}

// 창 합성 캡처를 논리(CSS px, 창 좌표) rect 로 crop 해 PNG 바이트로 반환(디스크 미경유).
// rect=None 이면 창 전체. crop 은 CGImageCreateWithImageInRect — 전체 재인코딩 없이 부분만 인코딩.
pub(crate) async fn capture_region_png<R: Runtime>(
    win: &Webview<R>,
    rect: Option<(f64, f64, f64, f64)>,
) -> Result<Vec<u8>, String> {
    let info = fetch_window_info(win).await?;
    let crop = match rect {
        None => None,
        Some((x, y, w, h)) => Some(
            crate::commands::crop_rect_px(x, y, w, h, info.scale, info.width, info.height)
                .ok_or("빈/무효 crop rect")?,
        ),
    };
    let out = capture_with_info(info, Sink::Bytes { crop }).await?;
    out.ok_or_else(|| "캡처 바이트 없음".into())
}

async fn capture_sink<R: Runtime>(win: &Webview<R>, sink: Sink) -> Result<Option<Vec<u8>>, String> {
    let info = fetch_window_info(win).await?;
    capture_with_info(info, sink).await
}

// 1) 메인스레드: 대상 창의 CGWindowID + 픽셀 크기(frame × backingScale) + 배율.
async fn fetch_window_info<R: Runtime>(win: &Webview<R>) -> Result<WindowInfo, String> {
    use std::sync::mpsc;
    use std::time::Duration;
    let (tx_info, rx_info) = mpsc::sync_channel::<Result<WindowInfo, String>>(1);
    win.with_webview(move |pw| {
        let r = unsafe { window_info(pw.inner() as *mut AnyObject) };
        let _ = tx_info.try_send(r);
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        rx_info
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "창 정보 조회 시간 초과".to_string())?
    })
    .await
    .map_err(|e| e.to_string())?
}

// 2) ScreenCaptureKit 캡처. 완료 핸들러(getShareableContent → captureImage) 체인을 채널로 받는다.
//    SCK 가 항상 완료를 호출하므로 채널은 막히지 않는다(timeout 은 만일의 stuck 대비 backstop 일 뿐 —
//    렌더 중 hang 의 근본 해결은 블로킹 CGWindowListCreateImage 를 버린 것).
async fn capture_with_info(info: WindowInfo, sink: Sink) -> Result<Option<Vec<u8>>, String> {
    use std::sync::mpsc;
    use std::time::Duration;
    let (tx, rx) = mpsc::sync_channel::<Result<Option<Vec<u8>>, String>>(1);
    capture_via_sck(info, sink, tx);
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(8))
            .map_err(|_| "snapshot 시간 초과".to_string())?
    })
    .await
    .map_err(|e| e.to_string())?
}

struct WindowInfo {
    id: u32,
    width: usize,
    height: usize,
    scale: f64,
}

// 메인 스레드(with_webview 클로저)에서 동기 실행. wk = WKWebView → NSWindow.
unsafe fn window_info(wk: *mut AnyObject) -> Result<WindowInfo, String> {
    use objc2::msg_send;
    let nswindow: *mut AnyObject = msg_send![&*wk, window];
    if nswindow.is_null() {
        return Err("NSWindow 없음".into());
    }
    let num: isize = msg_send![&*nswindow, windowNumber];
    if num <= 0 {
        return Err("windowNumber 무효".into());
    }
    let scale: f64 = msg_send![&*nswindow, backingScaleFactor];
    let frame: NSRect = msg_send![&*nswindow, frame];
    let scale = if scale > 0.0 { scale } else { 1.0 };
    Ok(WindowInfo {
        id: num as u32,
        width: ((frame.size.width * scale).round() as usize).max(1),
        height: ((frame.size.height * scale).round() as usize).max(1),
        scale,
    })
}

// SCK 비동기 캡처 체인 — 호출 즉시 반환, 완료 시 tx 로 결과. getShareableContent → 창 매칭 → 단일창
// 필터 → captureImage(CGImage) → sink(파일 저장 | crop 후 PNG 바이트). 블록은 프레임워크가
// 복사·보관하므로 RcBlock 이 즉시 drop 돼도 안전.
fn capture_via_sck(
    info: WindowInfo,
    sink: Sink,
    tx: std::sync::mpsc::SyncSender<Result<Option<Vec<u8>>, String>>,
) {
    let handler = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        if content.is_null() || !err.is_null() {
            let _ = tx.try_send(Err("getShareableContent 실패".into()));
            return;
        }
        let content = unsafe { &*content };
        let windows = unsafe { content.windows() };
        let mut target: Option<Retained<SCWindow>> = None;
        for w in &windows {
            if unsafe { w.windowID() } == info.id {
                target = Some(w);
                break;
            }
        }
        let Some(w) = target else {
            let _ = tx.try_send(Err(format!("창 {} 못 찾음(권한?)", info.id)));
            return;
        };
        let filter =
            unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &w) };
        let cfg = unsafe { SCStreamConfiguration::new() };
        unsafe {
            cfg.setWidth(info.width);
            cfg.setHeight(info.height);
        }
        let tx2 = tx.clone();
        let sink2 = match &sink {
            Sink::File(p) => Sink::File(p.clone()),
            Sink::Bytes { crop } => Sink::Bytes { crop: *crop },
        };
        let cap = RcBlock::new(move |img: *mut CGImage, err2: *mut NSError| {
            if img.is_null() || !err2.is_null() {
                let _ = tx2.try_send(Err("captureImage 실패".into()));
                return;
            }
            let r = match &sink2 {
                Sink::File(path) => unsafe { cgimage_to_png(img, path).map(|_| None) },
                Sink::Bytes { crop } => unsafe { cgimage_crop_png_data(img, *crop).map(Some) },
            };
            let _ = tx2.try_send(r);
        });
        unsafe {
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                &filter,
                &cfg,
                Some(&cap),
            );
        }
    });
    // getCurrentProcess… = 이 프로세스 자기 창만(자기 캡처). getShareableContent 와 달리 Screen Recording
    // TCC 권한이 불필요하다(구 CGWindowListCreateImage 가 자기 창엔 권한 없이 됐던 동작을 SCK 로 유지).
    unsafe { SCShareableContent::getCurrentProcessShareableContentWithCompletionHandler(&handler) };
}

// CGImage → NSBitmapImageRep → PNG NSData.
unsafe fn cgimage_png_nsdata(
    cg: *mut CGImage,
) -> Result<Retained<objc2_foundation::NSData>, String> {
    use objc2::{class, msg_send};
    use objc2_app_kit::NSBitmapImageFileType;
    use objc2_foundation::NSData;

    let alloc: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
    let rep_raw: *mut AnyObject = msg_send![alloc, initWithCGImage: cg];
    let rep = Retained::<AnyObject>::from_raw(rep_raw).ok_or("NSBitmapImageRep nil")?;

    let png: Option<Retained<NSData>> = msg_send![
        &*rep,
        representationUsingType: NSBitmapImageFileType::PNG,
        properties: std::ptr::null::<AnyObject>()
    ];
    png.ok_or_else(|| "PNG 인코딩 실패".into())
}

// CGImage → PNG 파일.
unsafe fn cgimage_to_png(cg: *mut CGImage, out: &str) -> Result<(), String> {
    use objc2::msg_send;
    use objc2_foundation::NSString;

    let png = cgimage_png_nsdata(cg)?;
    let nspath = NSString::from_str(out);
    let ok: bool = msg_send![&*png, writeToFile: &*nspath, atomically: true];
    if ok {
        Ok(())
    } else {
        Err("파일 쓰기 실패".into())
    }
}

// CGImage → (crop 시 부분만) PNG 바이트. crop 은 물리 px(이미지 좌표, 원점 좌상단) —
// CGImageCreateWithImageInRect 라 전체 재인코딩 없이 부분만 인코딩한다.
unsafe fn cgimage_crop_png_data(
    cg: *mut CGImage,
    crop: Option<(f64, f64, f64, f64)>,
) -> Result<Vec<u8>, String> {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};

    let cropped;
    let target: *mut CGImage = match crop {
        None => cg,
        Some((x, y, w, h)) => {
            let rect = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
            cropped = CGImage::with_image_in_rect(Some(&*cg), rect).ok_or("crop 실패")?;
            let ptr: *const CGImage = &*cropped;
            ptr as *mut CGImage
        }
    };
    let png = cgimage_png_nsdata(target)?;
    Ok(png.to_vec())
}
