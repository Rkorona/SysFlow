use crate::config::Config;
use crate::ctx::Ctx;

/// Wire reason string for the out-of-band rescue marker. Keep in one place: boot, enforcement and
/// the daemon all report this value to the CLI/WebUI/APK.
pub const RESCUE_REASON: &str = "disable_tuning";
/// Wire reason string for the persisted safe-mode flag.
pub const SAFE_MODE_REASON: &str = "safe_mode";

/// A file named `disable_tuning` in either the data dir or the module dir makes
/// `apply-boot` do nothing. Users can create it from recovery/adb without the APK.
pub fn rescue_marker_present(ctx: &Ctx) -> bool {
    ctx.data_dir.join("disable_tuning").exists() || ctx.module_dir.join("disable_tuning").exists()
}

/// The single "automatic tuning must not run" gate: the out-of-band rescue marker wins over the
/// persisted safe-mode flag, so a stale `safe_mode:false` can never silence the marker.
///
/// Boot and enforcement call [`rescue_marker_present`] *before* loading the config as well: the
/// marker exists for the case where the config cannot be trusted, so a config that fails to load
/// must not pre-empt the rescue path.
pub fn block_reason(cfg: &Config, ctx: &Ctx) -> Option<&'static str> {
    if rescue_marker_present(ctx) {
        return Some(RESCUE_REASON);
    }
    cfg.safe.safe_mode.then_some(SAFE_MODE_REASON)
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
        return BootDecision::Skip(SAFE_MODE_REASON);
    }
    if c.safe.boot_count >= c.safe.threshold {
        c.safe.safe_mode = true;
        return BootDecision::Skip(SAFE_MODE_REASON);
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
    use crate::sysfs::Sysfs;

    /// A ctx with distinct data/module dirs so both marker locations can be tested separately.
    fn temp_ctx(name: &str) -> Ctx {
        let root = std::env::temp_dir().join(format!("sysflow-safe-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data_dir = root.join("data");
        let module_dir = root.join("module");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&module_dir).unwrap();
        Ctx {
            data_dir,
            module_dir,
            sysfs: Sysfs::new(root.join("sysfs")),
        }
    }

    #[test]
    fn rescue_marker_wins_over_safe_mode() {
        let ctx = temp_ctx("marker-priority");
        let mut c = Config::default();
        c.safe.safe_mode = true;
        assert_eq!(
            block_reason(&c, &ctx),
            Some(SAFE_MODE_REASON),
            "no marker: safe mode is the reported reason"
        );

        std::fs::write(ctx.data_dir.join("disable_tuning"), "").unwrap();
        assert_eq!(
            block_reason(&c, &ctx),
            Some(RESCUE_REASON),
            "the rescue marker must win over safe mode"
        );

        // The marker is honoured in the module dir too (it survives a data wipe).
        std::fs::remove_file(ctx.data_dir.join("disable_tuning")).unwrap();
        std::fs::write(ctx.module_dir.join("disable_tuning"), "").unwrap();
        assert_eq!(block_reason(&c, &ctx), Some(RESCUE_REASON));
    }

    #[test]
    fn safe_mode_blocks_tuning() {
        let ctx = temp_ctx("safe-mode");
        let mut c = Config::default();
        assert!(!rescue_marker_present(&ctx));
        c.safe.safe_mode = true;
        assert_eq!(block_reason(&c, &ctx), Some(SAFE_MODE_REASON));
    }

    #[test]
    fn no_block_without_marker_or_safe_mode() {
        let ctx = temp_ctx("no-block");
        assert_eq!(block_reason(&Config::default(), &ctx), None);
    }

    #[test]
    fn enters_safe_mode_after_threshold_unconfirmed_boots() {
        let mut c = Config::default();
        for _ in 0..3 {
            assert_eq!(begin_boot(&mut c), BootDecision::Proceed);
        }
        assert_eq!(c.safe.boot_count, 3);
        assert_eq!(begin_boot(&mut c), BootDecision::Skip(SAFE_MODE_REASON));
        assert!(c.safe.safe_mode);
        // stays in safe mode until cleared
        assert_eq!(begin_boot(&mut c), BootDecision::Skip(SAFE_MODE_REASON));
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
