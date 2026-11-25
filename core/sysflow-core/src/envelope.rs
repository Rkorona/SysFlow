use serde_json::{json, Value};

use crate::error::Error;

/// Protocol major version. Only bumped on incompatible changes.
pub const PROTOCOL_VERSION: u32 = 1;

pub fn ok(data: Value) -> Value {
    json!({ "v": PROTOCOL_VERSION, "ok": true, "data": data })
}

pub fn err(e: &Error) -> Value {
    let mut o = json!({ "code": e.code.as_str(), "msg": e.msg });
    if let Some(d) = &e.detail {
        o["detail"] = d.clone();
    }
    json!({ "v": PROTOCOL_VERSION, "ok": false, "error": o })
}
