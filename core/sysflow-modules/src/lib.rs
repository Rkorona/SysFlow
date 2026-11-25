//! Concrete feature modules. The mock module is for host-side testing;
//! the CPU module (phase 1) is the first real one and the battery module the first read-only one.

pub mod battery;
pub mod cpu;
pub mod mem;
pub mod mock;
pub mod wakelock;

use sysflow_core::ctx::Ctx;
use sysflow_core::module::Module;

/// All modules compiled into this binary. The mock module is only registered when
/// `SYSFLOW_MOCK=1`, so it never shows up on a real device.
pub fn builtin(ctx: &Ctx) -> Vec<Box<dyn Module>> {
    let mut v: Vec<Box<dyn Module>> = vec![
        Box::new(cpu::CpuModule::with_state_dir(
            ctx.sysfs.clone(),
            ctx.data_dir.join("state"),
        )),
        Box::new(battery::BatteryModule::new(ctx.sysfs.clone())),
        Box::new(wakelock::WakelockModule::new(
            ctx.sysfs.clone(),
            ctx.data_dir.join("state"),
        )),
        Box::new(mem::MemModule::with_state_dir(
            ctx.sysfs.clone(),
            ctx.data_dir.join("state"),
        )),
    ];
    if std::env::var("SYSFLOW_MOCK")
        .map(|s| s == "1")
        .unwrap_or(false)
    {
        v.push(Box::new(mock::MockModule::new(
            ctx.data_dir.join("mock_state.json"),
        )));
    }
    v
}
