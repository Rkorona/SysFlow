//! Foreground application detection via `dumpsys activity activities`.
//!
//! The resumed-activity line is the only stable, cheap source. It is parsed tolerantly because its
//! exact shape differs across Android releases (`topResumedActivity=` vs `mResumedActivity:`, with
//! or without the `u<user>` token). A successful lookup is cached for five seconds so a 2 s UI poll
//! does not call `dumpsys` on every refresh; a failed lookup is **not** cached, so the page recovers
//! as soon as the command works again.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use sysflow_core::cache;
use sysflow_core::dumpsys;

const TTL_SECS: u64 = 5;
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Foreground {
    pub available: bool,
    pub package: Option<String>,
    pub user: u32,
    pub cached: bool,
}

pub fn current(data_dir: &Path) -> Foreground {
    let path = data_dir.join("state").join("process_foreground.json");
    if let Some(cached) = cache::read_fresh(&path, TTL_SECS) {
        if let Some(package) = cached.get("package").and_then(Value::as_str) {
            return Foreground {
                available: true,
                package: Some(package.to_string()),
                user: cached.get("user").and_then(Value::as_u64).unwrap_or(0) as u32,
                cached: true,
            };
        }
        // A fresh "checked, nothing resumed" entry is a legitimate answer (e.g. the device is on the
        // lock screen); it is cached for the same five seconds. Transport failures never get here.
        if cached
            .get("checked")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Foreground {
                available: false,
                package: None,
                user: 0,
                cached: true,
            };
        }
    }

    match dumpsys::run_with_timeout(&["activity", "activities"], TIMEOUT) {
        Ok(text) => {
            let parsed = parse_output(&text);
            cache::write(
                &path,
                &json!({
                    "generated_at": cache::now_secs(),
                    "checked": true,
                    "package": parsed.as_ref().map(|(package, _)| package.clone()),
                    "user": parsed.as_ref().map(|(_, user)| *user).unwrap_or(0),
                }),
            );
            match parsed {
                Some((package, user)) => Foreground {
                    available: true,
                    package: Some(package),
                    user,
                    cached: false,
                },
                None => Foreground {
                    available: false,
                    package: None,
                    user: 0,
                    cached: false,
                },
            }
        }
        // Do not cache: the caller may still render the list without a foreground highlight, and
        // the next poll should try again rather than inherit a 5 s false negative.
        Err(_) => Foreground {
            available: false,
            package: None,
            user: 0,
            cached: false,
        },
    }
}

/// `topResumedActivity` wins; `mResumedActivity` is the per-release alternative.
pub fn parse_output(text: &str) -> Option<(String, u32)> {
    let top = text
        .lines()
        .find(|line| line.contains("topResumedActivity"));
    if let Some(parsed) = top.and_then(parse_line) {
        return Some(parsed);
    }
    text.lines()
        .find(|line| line.contains("mResumedActivity"))
        .and_then(parse_line)
}

/// Extract `(package, user)` from a line such as
/// `topResumedActivity=ActivityRecord{e8a3f1 u0 com.android.settings/.Settings t42}`.
pub fn parse_line(line: &str) -> Option<(String, u32)> {
    let user = line
        .split_whitespace()
        .find(|token| {
            token.len() > 1
                && token.starts_with('u')
                && token[1..].chars().all(|c| c.is_ascii_digit())
        })
        .and_then(|token| token[1..].parse().ok())
        .unwrap_or(0);
    let token = line.split_whitespace().find(|token| {
        token.contains('/')
            && token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
    })?;
    let package = token.split('/').next()?;
    (!package.is_empty()).then(|| (package.to_string(), user))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_top_resumed_activity_shape() {
        let line = "    topResumedActivity=ActivityRecord{172310012 u0 com.server.auditor.ssh.client/.ssh.terminal.TerminalActivity t2919}";
        assert_eq!(
            parse_line(line),
            Some(("com.server.auditor.ssh.client".to_string(), 0))
        );
    }

    #[test]
    fn parses_the_m_resumed_activity_shape_and_a_user_prefix() {
        let line =
            "  mResumedActivity: ActivityRecord{1a2b3c u10 com.android.settings/.Settings t7}";
        assert_eq!(
            parse_line(line),
            Some(("com.android.settings".to_string(), 10))
        );
    }

    #[test]
    fn top_wins_but_falls_back_when_unparsable() {
        let text = "  topResumedActivity=ActivityRecord{oops u0 no-package-here t1}\n  mResumedActivity: ActivityRecord{aa u0 com.example/.Main t2}\n";
        assert_eq!(parse_output(text), Some(("com.example".to_string(), 0)));
    }

    #[test]
    fn no_activity_line_is_not_an_error() {
        assert_eq!(parse_output("only unrelated output"), None);
        assert_eq!(parse_line("no slash here"), None);
    }
}
