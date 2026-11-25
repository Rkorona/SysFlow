use crate::config::Config;
use crate::ctx::Ctx;

/// A file named `disable_tuning` in either the data dir or the module dir makes
/// `apply-boot` do nothing. Users can create it from recovery/adb without the APK.
pub fn rescue_marker_present(ctx: &Ctx) -> bool {
    ctx.data_dir.join("disable_tuning").exists() || ctx.module_dir.join("disable_tuning").exists()
}

#[derive(Debug, PartialEq, Eq)]
pub enum BootDecision {
    Proceed,
    Skip(&'static str),
}

/// Called at the start of apply-boot (under the config lock).
/// If `threshold` previous boots were never confirmed, enter safe mode instead of tuning again.
pub fn begin_boot(c: &mut Config) -> BootDecision {
    if c.safe.safe_mode {
        return BootDecision::Skip("safe_mode");
    }
    if c.safe.boot_count >= c.safe.threshold {
        c.safe.safe_mode = true;
        return BootDecision::Skip("safe_mode");
    }
    c.safe.boot_count += 1;
    BootDecision::Proceed
}

/// The system stayed up long enough: this boot is good.
pub fn confirm_boot(c: &mut Config) {
    c.safe.boot_count = 0;
}

/// User explicitly leaves safe mode.
pub fn clear(c: &mut Config) {
    c.safe.safe_mode = false;
    c.safe.boot_count = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enters_safe_mode_after_threshold_unconfirmed_boots() {
        let mut c = Config::default();
        for _ in 0..3 {
            assert_eq!(begin_boot(&mut c), BootDecision::Proceed);
        }
        assert_eq!(c.safe.boot_count, 3);
        assert_eq!(begin_boot(&mut c), BootDecision::Skip("safe_mode"));
        assert!(c.safe.safe_mode);
        // stays in safe mode until cleared
        assert_eq!(begin_boot(&mut c), BootDecision::Skip("safe_mode"));
        clear(&mut c);
        assert_eq!(begin_boot(&mut c), BootDecision::Proceed);
    }

    #[test]
    fn confirming_resets_the_counter() {
        let mut c = Config::default();
        begin_boot(&mut c);
        begin_boot(&mut c);
        confirm_boot(&mut c);
        assert_eq!(c.safe.boot_count, 0);
        assert_eq!(begin_boot(&mut c), BootDecision::Proceed);
    }
}
