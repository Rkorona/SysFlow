//! Which apps may not be blocked, and what counts as a package name.
//!
//! One authority for all of them: `validate_rules` refuses to block these packages, `reset` skips
//! them, `sanitize_boot` drops them, and the module publishes the list as `wakelock.protected` so
//! clients stop keeping their own copy (a client copy drifts, and a drifted copy either blocks a
//! protected app or disables a switch that would have worked).

/// Apps whose wake locks may not be blocked, regardless of what a client asks for.
///
/// Blocking one of these can drop calls, alarms or telephony state — the exact failure mode the plan
/// calls out. They are also skipped when releasing, so the module never flips a system default it
/// did not set.
pub const PROTECTED: &[&str] = &[
    "android",
    "com.android.systemui",
    "com.android.phone",
    "com.android.server.telecom",
    "com.android.dialer",
    "com.google.android.dialer",
    "com.android.mms",
    "com.android.messaging",
    "com.android.providers.telephony",
    "com.android.settings",
    "io.sysflow",
];

pub fn is_protected(package: &str) -> bool {
    PROTECTED.contains(&package)
}

/// A forgiving package-name check: dotted segments of letters, digits and underscores.
///
/// AppOps would reject garbage anyway, but validating first turns a confusing shell error into a
/// clear `invalid_arg` and keeps arbitrary input out of the command line.
pub fn is_package_name(package: &str) -> bool {
    if package.is_empty() || package.len() > 200 {
        return false;
    }
    let mut segments = 0;
    for segment in package.split('.') {
        if segment.is_empty() || segment.len() > 100 {
            return false;
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return false;
        }
        segments += 1;
    }
    segments >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_apps_are_recognised() {
        assert!(is_protected("android"));
        assert!(is_protected("com.android.systemui"));
        assert!(is_protected("io.sysflow"));
        assert!(!is_protected("com.example.app"));
    }

    #[test]
    fn rejects_unsafe_package_names() {
        assert!(is_package_name("com.example.app"));
        assert!(is_package_name("io.sysflow"));
        assert!(!is_package_name("com.example; rm -rf /"));
        assert!(!is_package_name("android"));
        assert!(!is_package_name("com..foo"));
        assert!(!is_package_name(""));
    }

    #[test]
    fn the_list_itself_is_sane() {
        // A malformed entry would silently protect nothing (or worse, everything).
        for package in PROTECTED {
            assert!(
                is_package_name(package) || *package == "android",
                "unexpected entry: {package}"
            );
        }
        assert!(PROTECTED.contains(&"com.android.phone"));
    }
}
