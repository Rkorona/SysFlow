//! AppOps `WAKE_LOCK` control.
//!
//! Blocking an app's wake locks is `cmd appops set <pkg> WAKE_LOCK ignore`; the read side is
//! `cmd appops query-op WAKE_LOCK ignore`, which returns **one command's worth of answers for every
//! app** instead of one command per app. That matters: `drift()` runs on the guard interval, so a
//! per-package loop would spawn dozens of binder calls every 10 seconds.
//!
//! The runner is injectable so the module can be tested on a host with no Android tooling. Tests
//! pass a closure backed by a `Mutex<HashSet>`; production passes [`real_runner`].

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use sysflow_core::command;
use sysflow_core::error::{Error, Result};

use super::policy::is_package_name;

/// How long an `appops` call may take.
const APOPS_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `cmd` with the given arguments and returns stdout.
pub type Runner = dyn Fn(&[&str]) -> Result<String> + Send + Sync;

pub fn real_runner() -> Arc<Runner> {
    Arc::new(|args: &[&str]| command::run("cmd", args, APOPS_TIMEOUT))
}

/// An AppOps mode this module writes.
///
/// `Default` is the *restore* mode: writing `allow` would leave a permanent explicit grant that
/// overrides the app's manifest and any OEM policy, so "unblock" would not actually restore
/// anything. `ignore` is the block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpMode {
    Ignore,
    Default,
}

impl OpMode {
    pub fn as_arg(self) -> &'static str {
        match self {
            OpMode::Ignore => "ignore",
            OpMode::Default => "default",
        }
    }
}

pub struct AppOps {
    runner: Arc<Runner>,
}

impl AppOps {
    pub fn real() -> Self {
        AppOps {
            runner: real_runner(),
        }
    }

    pub fn with_runner(runner: Arc<Runner>) -> Self {
        AppOps { runner }
    }

    /// Packages whose `WAKE_LOCK` op is currently `ignore`.
    pub fn query_blocked(&self) -> Result<BTreeSet<String>> {
        let text = (self.runner)(&["appops", "query-op", "WAKE_LOCK", "ignore"])?;
        Ok(parse_query_op(&text))
    }

    /// Block (`ignore`) or restore (`default`) one package.
    ///
    /// Releasing writes `default`, never `allow`: an explicit `allow` is a permanent grant that
    /// overrides the app's manifest and any OEM policy, which changes the app instead of restoring
    /// it.
    pub fn set_blocked(&self, package: &str, blocked: bool) -> Result<()> {
        if !is_package_name(package) {
            return Err(Error::invalid_arg(format!(
                "not a package name: {package:?}"
            )));
        }
        let mode = if blocked {
            OpMode::Ignore
        } else {
            OpMode::Default
        };
        (self.runner)(&["appops", "set", package, "WAKE_LOCK", mode.as_arg()])?;
        Ok(())
    }

    /// Whether the package is installed. On a failed query the answer is "yes", because dropping a
    /// user's rule because `pm` was slow would be worse than replaying it and reporting a mismatch.
    pub fn package_exists(&self, package: &str) -> bool {
        let Ok(text) = (self.runner)(&["package", "list", "packages", package]) else {
            return true;
        };
        parse_package_list(&text, package)
    }
}

/// Parse `cmd appops query-op` output.
///
/// Observed builds print one package per line; some prefix a uid or use `package=`. Anything that is
/// not a bare package name after that cleanup is skipped rather than guessed at.
pub fn parse_query_op(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let candidate = line.strip_prefix("package=").unwrap_or(line);
        // A uid may prefix the name (`1000:com.bar`), and some builds append the mode
        // (`com.foo: ignore`), so take the first token that actually looks like a package.
        let chosen = candidate
            .split([':', ' ', '\t'])
            .map(str::trim)
            .find(|token| is_package_name(token));
        if let Some(package) = chosen {
            out.insert(package.to_string());
        }
    }
    out
}

/// Exact match in `cmd package list packages <filter>` output (`package:com.foo`), because the
/// filter is a substring match and would otherwise accept `com.foobar` for `com.foo`.
pub fn parse_package_list(text: &str, package: &str) -> bool {
    let expected = format!("package:{package}");
    text.lines().any(|line| line.trim() == expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[test]
    fn parses_plain_and_decorated_query_output() {
        let text = "com.android.chrome\npackage=com.foo\n1000:com.bar\n  \ngarbage-line\n";
        let parsed = parse_query_op(text);
        assert!(parsed.contains("com.android.chrome"));
        assert!(parsed.contains("com.foo"));
        assert!(parsed.contains("com.bar"));
        assert!(!parsed.contains("garbage-line"), "not a package name");
    }

    #[test]
    fn package_list_requires_an_exact_match() {
        let text = "package:com.foo\npackage:com.foobar\n";
        assert!(parse_package_list(text, "com.foo"));
        assert!(!parse_package_list(text, "com.fo"));
        assert!(!parse_package_list(text, "com.baz"));
    }

    /// A fake `cmd` that keeps an in-memory ignore set, so apply/read-back can be tested honestly.
    fn fake_runner(
        state: Arc<Mutex<HashSet<String>>>,
        installed: Vec<&'static str>,
    ) -> Arc<Runner> {
        let installed: Vec<String> = installed.into_iter().map(str::to_string).collect();
        Arc::new(move |args: &[&str]| -> Result<String> {
            match args {
                ["appops", "query-op", "WAKE_LOCK", "ignore"] => {
                    let set = state.lock().unwrap();
                    Ok(set.iter().cloned().collect::<Vec<_>>().join("\n"))
                }
                ["appops", "set", package, "WAKE_LOCK", mode] => {
                    let mut set = state.lock().unwrap();
                    if *mode == "ignore" {
                        set.insert(package.to_string());
                    } else {
                        set.remove(*package);
                    }
                    Ok(String::new())
                }
                ["package", "list", "packages", filter] => {
                    let matching: Vec<String> = installed
                        .iter()
                        .filter(|p| p.contains(filter))
                        .map(|p| format!("package:{p}"))
                        .collect();
                    Ok(matching.join("\n"))
                }
                other => Err(Error::io(format!("unexpected command: {other:?}"))),
            }
        })
    }

    #[test]
    fn blocking_writes_ignore_and_releasing_writes_default() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let runner: Arc<Runner> = {
            let seen = Arc::clone(&seen);
            Arc::new(move |args: &[&str]| {
                if let ["appops", "set", _, "WAKE_LOCK", mode] = args {
                    seen.lock().unwrap().push((*mode).to_string());
                }
                Ok(String::new())
            })
        };
        let appops = AppOps::with_runner(runner);
        appops.set_blocked("com.example.app", true).unwrap();
        appops.set_blocked("com.example.app", false).unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["ignore".to_string(), "default".to_string()],
            "releasing must restore the AppOps default, not grant an explicit allow"
        );
    }

    #[test]
    fn set_and_query_round_trip_through_the_runner() {
        let state = Arc::new(Mutex::new(HashSet::new()));
        let appops = AppOps::with_runner(fake_runner(Arc::clone(&state), vec!["com.example.app"]));

        assert!(appops.query_blocked().unwrap().is_empty());
        appops.set_blocked("com.example.app", true).unwrap();
        assert!(appops.query_blocked().unwrap().contains("com.example.app"));
        appops.set_blocked("com.example.app", false).unwrap();
        assert!(appops.query_blocked().unwrap().is_empty());
    }

    #[test]
    fn set_rejects_a_bad_package_before_running_anything() {
        let state = Arc::new(Mutex::new(HashSet::new()));
        let appops = AppOps::with_runner(fake_runner(state, vec![]));
        let err = appops
            .set_blocked("com.foo && reboot", true)
            .expect_err("must refuse");
        assert_eq!(err.code.as_str(), "invalid_arg");
    }

    #[test]
    fn package_existence_is_exact() {
        let state = Arc::new(Mutex::new(HashSet::new()));
        let appops = AppOps::with_runner(fake_runner(
            state,
            vec!["com.example.app", "com.example.app2"],
        ));
        assert!(appops.package_exists("com.example.app"));
        assert!(!appops.package_exists("com.example.other"));
    }

    #[test]
    fn package_existence_fails_open_when_the_query_errors() {
        let runner: Arc<Runner> = Arc::new(|_args: &[&str]| Err(Error::io("pm unavailable")));
        let appops = AppOps::with_runner(runner);
        assert!(
            appops.package_exists("com.example.app"),
            "a failing query must not drop the user's rule"
        );
    }
}
