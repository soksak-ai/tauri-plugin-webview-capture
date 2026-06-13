use serde::{Serialize, Serializer};

/// 플러그인 명령 에러. 프론트로 직렬화될 때 문자열 메시지가 된다.
#[derive(Debug)]
pub enum Error {
    /// 네이티브 캡처/토글 실패(플랫폼 메시지).
    Capture(String),
    /// 파일/디렉터리 IO 실패.
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Capture(s) => write!(f, "{s}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

// Tauri 명령은 Err 값을 직렬화해 프론트로 보낸다 — 메시지 문자열로.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
