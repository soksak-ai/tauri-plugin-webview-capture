// Windows 캡처 — WebView2 ICoreWebView2.CapturePreview(PNG) → IStream → 파일.
//
// 컴파일은 소비 앱의 3-OS CI 게이트(windows-2025 cargo check)로 검증된다 —
// CapturePreviewCompletedHandler 콜백은 HRESULT 가 아니라 windows::core::Result<()>
// 를 받는다(첫 실검사가 드러낸 정정). 런타임 캡처 동작은 Windows 실기 검증이 남아 있다.
// 크레이트 버전은 tauri 2.11.x(wry)의 Cargo.lock 과 일치하도록 핀(0.38/0.61) —
// controller()/windows 버전이 갈리면 타입 불일치(cargo tree -d 로 webview2-com 중복 없어야).
use tauri::{Runtime, Webview};

pub(crate) async fn capture<R: Runtime>(win: &Webview<R>, path: &str) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    // 콜백은 PNG 바이트(또는 에러 문자열)를 보낸다.
    let (tx, rx) = mpsc::sync_channel::<Result<Vec<u8>, String>>(1);

    win.with_webview(move |pw| {
        use webview2_com::CapturePreviewCompletedHandler;
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
        use windows::Win32::System::Com::IStream;
        use windows::Win32::UI::Shell::SHCreateMemStream;

        // with_webview 클로저는 UI(메인) 스레드에서 실행된다(tauri 보장) — 막으면
        // 메시지 펌프가 멈춰 콜백이 영영 안 온다. CapturePreview 만 발사하고 결과는
        // 콜백이 sync_channel 로 보낸다(macOS 와 동일 구조).
        let outcome = (|| -> Result<(), String> {
            unsafe {
                let controller = pw.controller();
                let core = controller
                    .CoreWebView2()
                    .map_err(|e| format!("CoreWebView2 획득 실패: {e}"))?;
                // 빈 메모리 IStream — CapturePreview 가 여기에 PNG 를 쓴다.
                let stream: IStream =
                    SHCreateMemStream(None).ok_or("IStream 생성 실패(SHCreateMemStream)")?;
                let tx = tx.clone();
                let cb_stream = stream.clone();
                // webview2-com 의 CapturePreviewCompletedHandler 콜백은 HRESULT 가 아니라
                // windows::core::Result<()>(성공/변환된 에러)를 받는다(cargo check 실측 — 기대
                // 시그니처 fn(Result<(), Error>)). 성공이면 스트림을 읽는다.
                let handler = CapturePreviewCompletedHandler::create(Box::new(
                    move |status: windows::core::Result<()>| -> windows::core::Result<()> {
                        let result = (|| -> Result<Vec<u8>, String> {
                            status.map_err(|e| format!("CapturePreview 실패: {e}"))?;
                            read_istream_all(&cb_stream)
                                .map_err(|e| format!("스트림 읽기 실패: {e}"))
                        })();
                        let _ = tx.try_send(result);
                        Ok(())
                    },
                ));
                core.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
                .map_err(|e| format!("CapturePreview 호출 실패: {e}"))?;
            }
            Ok(())
        })();
        // 발사 자체가 실패하면(콜백이 안 옴) 즉시 에러를 보낸다.
        if let Err(e) = outcome {
            let _ = tx.try_send(Err(e));
        }
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

// IStream 전체를 바이트로: 끝까지 쓰인 스트림을 처음으로 되감고(Seek SET=0) Stat
// 크기만큼 Read. STREAM_SEEK/STATFLAG 는 공개 newtype 이라 상수명에 기대지 않고 0 으로 구성.
fn read_istream_all(
    stream: &windows::Win32::System::Com::IStream,
) -> windows::core::Result<Vec<u8>> {
    use windows::Win32::System::Com::{STATFLAG, STATSTG, STREAM_SEEK};
    unsafe {
        stream.Seek(0, STREAM_SEEK(0), None)?; // dworigin=STREAM_SEEK_SET
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG(0))?; // STATFLAG_DEFAULT
        let size = stat.cbSize as usize;
        let mut buf = vec![0u8; size];
        let mut total = 0usize;
        // 한 번에 다 안 읽힐 수 있어 채울 때까지 반복.
        while total < size {
            let mut read: u32 = 0;
            let hr = stream.Read(
                buf.as_mut_ptr().add(total) as *mut std::ffi::c_void,
                (size - total) as u32,
                Some(&mut read),
            );
            hr.ok()?; // S_OK/S_FALSE 모두 통과
            if read == 0 {
                break;
            }
            total += read as usize;
        }
        buf.truncate(total);
        Ok(buf)
    }
}

// 가림 감지: Windows 엔 macOS 식 occlusion 스로틀이 없다(최소화/숨김 때만 멈춤) → no-op.
// put_IsVisible 는 가시성 제어라 캡처와 반대 방향 — 안 건드린다.
pub(crate) fn set_occlusion<R: Runtime>(_win: &Webview<R>, _enabled: bool) -> Result<(), String> {
    Ok(())
}

// 캡처 직전 준비(비-macOS). 끌 가림감지는 없지만 흐름을 macOS 와 동일하게 유지하고
// 직전 레이아웃 변화가 렌더에 반영될 여유(200ms)를 준다.
pub(crate) async fn arm_capture<R: Runtime>(_win: &Webview<R>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(200))
    })
    .await
    .map_err(|e| e.to_string())
}

pub(crate) fn disarm_capture<R: Runtime>(_win: &Webview<R>) {}

// snapshot_region — WebView2 CapturePreview 의 rect crop 은 아직 미구현(파일 스냅샷만 지원).
pub(crate) async fn capture_region_png<R: Runtime>(
    _win: &Webview<R>,
    _rect: Option<(f64, f64, f64, f64)>,
) -> Result<Vec<u8>, String> {
    Err("snapshot_region: Windows 미구현".into())
}
