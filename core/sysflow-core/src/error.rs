use std::fmt;

use serde_json::Value;

/// Fixed protocol error codes. The UI only has to handle these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Unsupported,
    Permission,
    InvalidArg,
    Busy,
    Io,
    SafeMode,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Permission => "permission",
            ErrorCode::InvalidArg => "invalid_arg",
            ErrorCode::Busy => "busy",
            ErrorCode::Io => "io",
            ErrorCode::SafeMode => "safe_mode",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub code: ErrorCode,
    pub msg: String,
    pub detail: Option<Value>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: ErrorCode, msg: impl Into<String>) -> Self {
        Error {
            code,
            msg: msg.into(),
            detail: None,
        }
    }
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, msg)
    }
    pub fn permission(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Permission, msg)
    }
    pub fn invalid_arg(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArg, msg)
    }
    pub fn busy(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Busy, msg)
    }
    pub fn io(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Io, msg)
    }
    pub fn safe_mode(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::SafeMode, msg)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.msg)
    }
}

impl std::error::Error for Error {}

/// Map an io::Error to a protocol error, with a context prefix (usually a path).
pub fn from_io(e: &std::io::Error, ctx: &str) -> Error {
    let msg = if ctx.is_empty() {
        e.to_string()
    } else {
        format!("{ctx}: {e}")
    };
    match e.kind() {
        std::io::ErrorKind::NotFound => Error::unsupported(msg),
        std::io::ErrorKind::PermissionDenied => Error::permission(msg),
        std::io::ErrorKind::InvalidInput => Error::invalid_arg(msg),
        _ => {
            // EBUSY == 16 on Linux/Android
            if e.raw_os_error() == Some(16) {
                Error::busy(msg)
            } else {
                Error::io(msg)
            }
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        from_io(&e, "")
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::invalid_arg(format!("invalid JSON: {e}"))
    }
}
