use crate::error::{Error, Result};
use crate::platform;
use tauri::{Runtime, Webview};

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
    webview_window: Webview<R>,
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
    webview_window: Webview<R>,
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

/// 호출한 창의 합성 이미지를 논리(CSS px, 창 좌표) rect 로 crop 해 base64 PNG 로 반환
/// (디스크 미경유). rect 생략 = 창 전체. 가림 상태에서도 캡처(snapshot 과 동일 arm/disarm).
/// 용도: 네이티브 표면 freeze-frame(드래그 중 시각 연속 스탠드인), 부분 눈검증.
#[tauri::command]
pub async fn snapshot_region<R: Runtime>(
    webview_window: Webview<R>,
    x: Option<f64>,
    y: Option<f64>,
    w: Option<f64>,
    h: Option<f64>,
) -> Result<String> {
    use base64::Engine as _;
    let rect = match (x, y, w, h) {
        (Some(x), Some(y), Some(w), Some(h)) => Some((x, y, w, h)),
        (None, None, None, None) => None,
        _ => return Err(Error::Capture("rect 는 x/y/w/h 전부 또는 전부 생략".into())),
    };
    platform::arm_capture(&webview_window).await.map_err(Error::Capture)?;
    let r = platform::capture_region_png(&webview_window, rect).await;
    platform::disarm_capture(&webview_window); // 항상 복원
    let bytes = r.map_err(Error::Capture)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 가림감지 토글(macOS). false 면 다른 앱에 완전히 가려져도 렌더를 멈추지 않는다
/// (상시 백그라운드 캡처용 — 배터리 비용 주의). Windows/Linux 엔 동등 스로틀이 없어
/// no-op. snapshot/record 는 캡처 순간만 자동으로 끄므로 평소엔 불필요.
#[tauri::command]
pub fn set_occlusion<R: Runtime>(webview_window: Webview<R>, enabled: bool) -> Result<()> {
    platform::set_occlusion(&webview_window, enabled).map_err(Error::Capture)
}

/// 분수 좌표(0..1) 직사각 영역. record 로 찍은 프레임 위에서 명도를 표본할 구획.
#[derive(serde::Deserialize)]
pub struct Region {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// `dir` 의 f0000.png.. 연속 프레임마다 각 영역의 평균 명도(luma 0..255)를 계산.
/// 반환 = frames × regions 그리드(없는 프레임에서 멈춤). 테마 전환 tear·시각 회귀를
/// 코드로 판정하기 위한 generic 비전 원시 — 테마를 모르고 픽셀만 본다(해석은 호출자 몫).
#[tauri::command]
pub async fn analyze_regions(dir: String, regions: Vec<Region>) -> Result<Vec<Vec<f64>>> {
    tauri::async_runtime::spawn_blocking(move || analyze_blocking(&dir, &regions))
        .await
        .map_err(|e| Error::Capture(format!("join: {e}")))?
}

fn analyze_blocking(dir: &str, regions: &[Region]) -> Result<Vec<Vec<f64>>> {
    let mut out: Vec<Vec<f64>> = Vec::new();
    let mut i = 0u32;
    loop {
        let p = format!("{dir}/f{i:04}.png");
        if !std::path::Path::new(&p).exists() {
            break;
        }
        let img = image::open(&p)
            .map_err(|e| Error::Capture(format!("{p}: {e}")))?
            .to_luma8();
        let (w, h) = img.dimensions();
        let row: Vec<f64> = regions
            .iter()
            .map(|r| region_mean(&img, w, h, r))
            .collect();
        out.push(row);
        i += 1;
    }
    Ok(out)
}

fn region_bounds(w: u32, h: u32, r: &Region) -> (u32, u32, u32, u32) {
    let c = |v: f64| v.clamp(0.0, 1.0);
    let x0 = (c(r.x0) * w as f64) as u32;
    let y0 = (c(r.y0) * h as f64) as u32;
    let x1 = ((c(r.x1) * w as f64) as u32).min(w);
    let y1 = ((c(r.y1) * h as f64) as u32).min(h);
    (x0, y0, x1, y1)
}

fn region_mean(img: &image::GrayImage, w: u32, h: u32, r: &Region) -> f64 {
    let (x0, y0, x1, y1) = region_bounds(w, h, r);
    let mut sum = 0u64;
    let mut cnt = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            sum += img.get_pixel(x, y)[0] as u64;
            cnt += 1;
        }
    }
    if cnt == 0 {
        0.0
    } else {
        sum as f64 / cnt as f64
    }
}

/// `dir` 의 연속 프레임마다, 각 영역에서 *직전 프레임과 다른* 픽셀(luma 차 > thresh)의 비율(0..1)을 계산.
/// 반환 = frames × regions(첫 프레임 행은 0). 콘텐츠 *전환*·애니메이션 같은 변화 감지용 — 밝기로는 구별
/// 안 되는 같은-색 콘텐츠끼리의 전환도 픽셀 차이로 잡는다(탭 전환이 단일 프레임에 끝나는지/번지는지 판정).
#[tauri::command]
pub async fn analyze_frame_diffs(
    dir: String,
    regions: Vec<Region>,
    thresh: Option<u8>,
) -> Result<Vec<Vec<f64>>> {
    let t = thresh.unwrap_or(30);
    tauri::async_runtime::spawn_blocking(move || frame_diffs_blocking(&dir, &regions, t))
        .await
        .map_err(|e| Error::Capture(format!("join: {e}")))?
}

fn frame_diffs_blocking(dir: &str, regions: &[Region], thresh: u8) -> Result<Vec<Vec<f64>>> {
    let mut out: Vec<Vec<f64>> = Vec::new();
    let mut prev: Option<image::GrayImage> = None;
    let mut i = 0u32;
    loop {
        let p = format!("{dir}/f{i:04}.png");
        if !std::path::Path::new(&p).exists() {
            break;
        }
        let img = image::open(&p)
            .map_err(|e| Error::Capture(format!("{p}: {e}")))?
            .to_luma8();
        let (w, h) = img.dimensions();
        let row: Vec<f64> = regions
            .iter()
            .map(|r| match &prev {
                None => 0.0,
                Some(pv) if pv.dimensions() != img.dimensions() => 1.0,
                Some(pv) => region_changed_frac(&img, pv, w, h, r, thresh),
            })
            .collect();
        out.push(row);
        prev = Some(img);
        i += 1;
    }
    Ok(out)
}

/// 논리(CSS px, 창 좌표) 직사각 → 물리(px) crop 사각. 이미지 경계로 클램프.
/// 폭/높이가 0 이하로 잘리면 None(빈 crop 은 캡처 실패보다 명시적 거절이 낫다).
pub(crate) fn crop_rect_px(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
    img_w: usize,
    img_h: usize,
) -> Option<(f64, f64, f64, f64)> {
    if !(scale > 0.0) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let x0 = (x * scale).max(0.0).min(img_w as f64);
    let y0 = (y * scale).max(0.0).min(img_h as f64);
    let x1 = ((x + w) * scale).max(0.0).min(img_w as f64);
    let y1 = ((y + h) * scale).max(0.0).min(img_h as f64);
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn region_changed_frac(
    cur: &image::GrayImage,
    prev: &image::GrayImage,
    w: u32,
    h: u32,
    r: &Region,
    thresh: u8,
) -> f64 {
    let (x0, y0, x1, y1) = region_bounds(w, h, r);
    let mut changed = 0u64;
    let mut cnt = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            if cur.get_pixel(x, y)[0].abs_diff(prev.get_pixel(x, y)[0]) > thresh {
                changed += 1;
            }
            cnt += 1;
        }
    }
    if cnt == 0 {
        0.0
    } else {
        changed as f64 / cnt as f64
    }
}

#[cfg(test)]
mod tests {
    use super::crop_rect_px;

    #[test]
    fn 논리를_배율로_물리화한다() {
        // 2x 디스플레이: 논리 (10,20,100,50) → 물리 (20,40,200,100)
        assert_eq!(
            crop_rect_px(10.0, 20.0, 100.0, 50.0, 2.0, 4000, 3000),
            Some((20.0, 40.0, 200.0, 100.0))
        );
    }

    #[test]
    fn 이미지_경계로_클램프한다() {
        // 오른쪽/아래로 벗어난 부분은 잘린다.
        assert_eq!(
            crop_rect_px(1900.0, 1000.0, 300.0, 300.0, 2.0, 4000, 2100),
            Some((3800.0, 2000.0, 200.0, 100.0))
        );
        // 음수 원점은 0 으로.
        assert_eq!(
            crop_rect_px(-10.0, -10.0, 20.0, 20.0, 1.0, 100, 100),
            Some((0.0, 0.0, 10.0, 10.0))
        );
    }

    #[test]
    fn 빈_또는_무효_crop_은_none() {
        assert_eq!(crop_rect_px(0.0, 0.0, 0.0, 10.0, 2.0, 100, 100), None); // w=0
        assert_eq!(crop_rect_px(0.0, 0.0, 10.0, 10.0, 0.0, 100, 100), None); // scale=0
        assert_eq!(crop_rect_px(200.0, 0.0, 10.0, 10.0, 1.0, 100, 100), None); // 완전 밖
    }
}
