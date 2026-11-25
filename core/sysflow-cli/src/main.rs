use std::io::Write;
use std::process::exit;

mod battery_apps;
mod battery_health;
mod mem_apps;
mod mem_breakdown;
mod mem_probe;
mod wakelock_apps;
mod wakelock_held;
mod wakelock_wakeup;

use clap::{Parser, Subcommand};
use serde_json::{json, Map, Value};

use sysflow_core::config::CONFIG_VERSION;
use sysflow_core::ctx::Ctx;
use sysflow_core::envelope::{self, PROTOCOL_VERSION};
use sysflow_core::event::{EventFilter, EventStore, NewSystemEvent};
use sysflow_core::registry::Registry;
use sysflow_core::{Error, Result, VERSION};

#[derive(Parser)]
#[command(
    name = "sysflow",
    version,
    about = "SysFlow device manager core (module CLI)"
)]
struct Cli {
    /// Pretty-print the JSON output
    #[arg(long, global = true)]
    pretty: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Binary, protocol and config versions
    Version,
    /// Per-module capability probe
    Caps,
    /// Current state of every supported module in one call
    Snapshot,
    /// Static SoC, board, kernel and Android build information.
    Info,
    /// Read-only drift check against persisted settings (used by the phase 2 daemon)
    Check,
    /// Recorded history for a module
    History {
        module: String,
        #[arg(long, default_value = "24h")]
        since: String,
        #[arg(long)]
        step: Option<String>,
    },
    /// Configure or inspect the boot-time apply behavior.
    Boot {
        #[command(subcommand)]
        action: BootAction,
    },
    /// Apply persisted settings at boot (called by service.sh)
    ApplyBoot {
        /// Skip the configured boot delay (tests)
        #[arg(long)]
        no_delay: bool,
    },
    /// Mark the current boot as good (called by service.sh after a delay)
    BootConfirm,
    /// Safe mode status / exit
    SafeMode {
        #[command(subcommand)]
        action: SafeAction,
    },
    /// Inspect or manage isolated recovery configuration backups.
    Recovery {
        #[command(subcommand)]
        action: RecoveryAction,
    },
    /// Background enforcement and adaptive sampling daemon.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Apply persisted settings that have drifted and advance adaptive CPU control once.
    #[command(hide = true)]
    Enforce,
    /// Append a short note to the module log (legacy compatibility).
    Note { message: String },
    /// Emit and query durable, module-owned structured system events.
    Event {
        #[command(subcommand)]
        action: EventAction,
    },
    /// Tail the module log
    Log {
        #[arg(long, default_value_t = 200)]
        tail: usize,
    },
    /// `sysflow reset --all` restores every module's defaults
    Reset {
        #[arg(long)]
        all: bool,
    },
    /// `sysflow <module> schema | get [--keys a,b] | set '<json>' [--persist] | reset`
    /// `sysflow battery apps [--limit N] [--refresh]` ranks per-app power attribution.
    #[command(external_subcommand)]
    Module(Vec<String>),
}

#[derive(Subcommand)]
enum EventAction {
    /// Emit one structured event from any current or future module.
    Emit {
        #[arg(long)]
        module: String,
        #[arg(long)]
        category: String,
        #[arg(long = "type")]
        event_type: String,
        #[arg(long, default_value = "info")]
        severity: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        message: String,
        #[arg(long, default_value = "{}")]
        metadata: String,
        #[arg(long)]
        dedupe_key: Option<String>,
        #[arg(long, default_value_t = 30)]
        dedupe_window_s: u64,
    },
    /// Read persisted events; filters are open-ended and do not depend on a module registry.
    List {
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        after_ms: Option<u64>,
        #[arg(long)]
        after_id: Option<String>,
        #[arg(long)]
        before_ms: Option<u64>,
        #[arg(long)]
        before_id: Option<String>,
        #[arg(long, default_value_t = false)]
        ascending: bool,
        #[arg(long)]
        module: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        severity: Option<String>,
    },
    /// Apply event journal retention policy.
    Prune {
        #[arg(long, default_value_t = 30)]
        retention_days: u64,
        #[arg(long, default_value_t = 5000)]
        max_events: usize,
    },
    /// Delete all module-side event records (requires --all explicitly).
    Clear {
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand)]
enum SafeAction {
    Status,
    Clear,
}

#[derive(Subcommand)]
enum RecoveryAction {
    Status,
    Restore,
    Clear,
    ClearCorrupt,
}

#[derive(Subcommand)]
enum BootAction {
    Status,
    Configure {
        #[arg(long)]
        enabled: Option<bool>,
        #[arg(long)]
        delay: Option<u32>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the detached enforcement / adaptive-sampling daemon.
    Start,
    /// Stop the running daemon.
    Stop,
    /// Show daemon state.
    Status,
    /// Configure automatic enforcement and its sampling interval.
    Configure {
        /// Enable or disable the background service.
        #[arg(long)]
        enabled: Option<bool>,
        /// Sampling/enforcement interval in seconds (2..=120).
        #[arg(long)]
        interval: Option<u64>,
    },
    /// Internal foreground loop (normally launched by `daemon start` or service.sh).
    Run,
}

fn flag_value<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    let pos = args.iter().position(|a| *a == flag)?;
    args.get(pos + 1).copied()
}

fn run_module(reg: &Registry, raw: &[String]) -> Result<Value> {
    let args: Vec<&str> = raw
        .iter()
        .map(String::as_str)
        .filter(|a| *a != "--pretty")
        .collect();
    let module = *args
        .first()
        .ok_or_else(|| Error::invalid_arg("missing module name"))?;
    let verb = *args
        .get(1)
        .ok_or_else(|| Error::invalid_arg("missing verb (schema | get | set | reset)"))?;
    let rest = &args[2..];
    match verb {
        "schema" => reg.schema(module),
        "get" => {
            let keys: Option<Vec<String>> = flag_value(rest, "--keys").map(|s| {
                s.split(',')
                    .filter(|k| !k.is_empty())
                    .map(str::to_string)
                    .collect()
            });
            reg.get(module, keys.as_deref())
        }
        "set" => {
            let payload = *rest
                .iter()
                .find(|a| !a.starts_with("--"))
                .ok_or_else(|| Error::invalid_arg("set requires a JSON payload"))?;
            let value: Value = serde_json::from_str(payload)?;
            reg.set(module, value, rest.contains(&"--persist"))
        }
        "reset" => reg.reset(module),
        // Re-seed the stored defaults from the kernel's own hardware floor.
        //
        // Defaults are captured once and never refreshed, so a config recorded before a kernel or
        // ROM update keeps a minimum frequency the hardware no longer honours — and `reset` keeps
        // restoring it. This records `cpuinfo_min_freq` (what the driver itself reports) as the
        // default minimum for every cluster that has one, which makes 恢复默认 mean "no artificial
        // floor" instead of "whatever the first write happened to see".
        "rebaseline" if module == "cpu" => {
            let hardware = reg.get(module, Some(&["cpu.hw_min_freq".to_string()]))?;
            let Some(by_cluster) = hardware.get("cpu.hw_min_freq").and_then(Value::as_object)
            else {
                return Err(Error::io("cpu reports no per-cluster hardware minimum"));
            };
            let floored: Map<String, Value> = by_cluster
                .iter()
                .filter(|(_, value)| value.as_i64().is_some_and(|freq| freq > 0))
                .map(|(cluster, value)| (cluster.clone(), value.clone()))
                .collect();
            if floored.is_empty() {
                return Err(Error::io(
                    "cpuinfo_min_freq is unreadable, so no hardware floor could be recorded",
                ));
            }
            let mut values = Map::new();
            values.insert("cpu.min_freq".to_string(), Value::Object(floored));
            reg.set_defaults(module, values)
        }
        // BatteryStats attribution is battery-specific and expensive, so it is a verb rather than
        // part of `snapshot`; the battery page pulls it lazily.
        "held" if module == "wakelock" => wakelock_held::run(reg, rest.contains(&"--refresh")),
        // Same cost as `held` (a `dumpsys power`), so also a cached verb rather than a snapshot item.
        "wakeup" if module == "wakelock" => wakelock_wakeup::run(reg, rest.contains(&"--refresh")),
        // BatteryStats wake-lock time is battery-scale history, so it stays out of `snapshot` for
        // the same reason `battery apps` does.
        "apps" if module == "wakelock" => {
            let limit = flag_value(rest, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(20);
            wakelock_apps::run(reg, limit, rest.contains(&"--refresh"))
        }
        // System-vs-user split: measures PSS for **every** process, which is why it is a verb and
        // not part of `snapshot`.
        "breakdown" if module == "mem" => mem_breakdown::run(reg, rest.contains(&"--refresh")),
        // Per-UID memory ranking: opens one /proc entry per process, so it is a verb, not a snapshot
        // item, and its result is cached for a minute.
        "apps" if module == "mem" => {
            let limit = flag_value(rest, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(mem_apps::DEFAULT_LIMIT);
            mem_apps::run(reg, limit, rest.contains(&"--refresh"))
        }
        // Hidden developer diagnostic: raw readings + the module's judgement; not in the protocol.
        "probe" if module == "mem" => mem_probe::run(reg),
        "health" if module == "battery" => battery_health::run(reg, rest.contains(&"--refresh")),
        "apps" if module == "battery" => {
            let limit = flag_value(rest, "--limit")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            battery_apps::run(reg, limit, rest.contains(&"--refresh"))
        }
        other => Err(Error::invalid_arg(format!("unknown verb: {other}"))),
    }
}

fn run(cli: &Cli, reg: &Registry, events: &EventStore) -> Result<Value> {
    match &cli.cmd {
        Cmd::Version => Ok(json!({
            "sysflow_version": VERSION,
            "protocol": PROTOCOL_VERSION,
            "config_version": CONFIG_VERSION,
        })),
        Cmd::Caps => reg.caps(),
        Cmd::Snapshot => reg.snapshot(),
        Cmd::Info => Ok(reg.info()),
        Cmd::Check => reg.check(),
        Cmd::History {
            module,
            since,
            step,
        } => reg.history(module, since, step.as_deref()),
        Cmd::Boot { action } => match action {
            BootAction::Status => reg.boot_status(),
            BootAction::Configure { enabled, delay } => reg.boot_configure(*enabled, *delay),
        },
        Cmd::ApplyBoot { no_delay } => reg.apply_boot(*no_delay),
        Cmd::BootConfirm => reg.boot_confirm(),
        Cmd::SafeMode { action } => match action {
            SafeAction::Status => reg.safe_status(),
            SafeAction::Clear => reg.safe_clear(),
        },
        Cmd::Recovery { action } => match action {
            RecoveryAction::Status => reg.recovery_status(),
            RecoveryAction::Restore => reg.recovery_restore(),
            RecoveryAction::Clear => reg.recovery_clear(),
            RecoveryAction::ClearCorrupt => reg.recovery_clear_corrupt(),
        },
        Cmd::Daemon { action } => match action {
            DaemonAction::Status => reg.daemon_status(),
            DaemonAction::Start => reg.daemon_start(),
            DaemonAction::Stop => reg.daemon_stop(),
            DaemonAction::Configure { enabled, interval } => {
                reg.daemon_configure(*enabled, *interval)
            }
            DaemonAction::Run => reg.daemon_run(),
        },
        Cmd::Enforce => reg.enforce(),
        Cmd::Note { message } => reg.note(message),
        Cmd::Event { action } => match action {
            EventAction::Emit {
                module,
                category,
                event_type,
                severity,
                title,
                message,
                metadata,
                dedupe_key,
                dedupe_window_s,
            } => {
                let metadata: Value = serde_json::from_str(metadata)
                    .map_err(|e| Error::invalid_arg(format!("invalid metadata JSON: {e}")))?;
                let (event, deduplicated) = events.emit(NewSystemEvent {
                    module: module.clone(),
                    category: category.clone(),
                    event_type: event_type.clone(),
                    severity: severity.clone(),
                    title: title.clone(),
                    message: message.clone(),
                    metadata,
                    dedupe_key: dedupe_key.clone(),
                    dedupe_window_s: *dedupe_window_s,
                })?;
                Ok(json!({ "event": event, "deduplicated": deduplicated }))
            }
            EventAction::List {
                limit,
                after_ms,
                after_id,
                before_ms,
                before_id,
                ascending,
                module,
                category,
                severity,
            } => {
                let records = events.list(EventFilter {
                    limit: *limit,
                    after_ms: *after_ms,
                    after_id: after_id.as_deref(),
                    before_ms: *before_ms,
                    before_id: before_id.as_deref(),
                    ascending: *ascending,
                    module: module.as_deref(),
                    category: category.as_deref(),
                    severity: severity.as_deref(),
                })?;
                let next_cursor_ms = records.last().map(|event| event.timestamp_ms);
                let next_cursor_id = records.last().map(|event| event.id.clone());
                Ok(json!({
                    "events": records,
                    "next_cursor_ms": next_cursor_ms,
                    "next_cursor_id": next_cursor_id,
                    "directory": events.directory().display().to_string(),
                }))
            }
            EventAction::Prune {
                retention_days,
                max_events,
            } => events.prune(*retention_days, *max_events),
            EventAction::Clear { all } => {
                if !all {
                    return Err(Error::invalid_arg("event clear requires --all"));
                }
                events.clear()
            }
        },
        Cmd::Log { tail } => reg.log_tail(*tail),
        Cmd::Reset { all } => {
            if *all {
                reg.reset_all()
            } else {
                Err(Error::invalid_arg(
                    "use `sysflow reset --all` or `sysflow <module> reset`",
                ))
            }
        }
        Cmd::Module(args) => run_module(reg, args),
    }
}

fn emit(value: &Value, pretty: bool) {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .unwrap_or_else(|_| String::from("{}"));
    // ignore broken pipes instead of panicking
    let _ = writeln!(std::io::stdout().lock(), "{text}");
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    exit(0);
                }
                _ => {
                    let msg = e.to_string();
                    emit(&envelope::err(&Error::invalid_arg(msg.trim())), false);
                    exit(2);
                }
            }
        }
    };

    let pretty =
        cli.pretty || matches!(&cli.cmd, Cmd::Module(a) if a.iter().any(|s| s == "--pretty"));

    let ctx = Ctx::from_env();
    let modules = sysflow_modules::builtin(&ctx);
    let reg = Registry::new(ctx.clone(), modules);
    let events = EventStore::new(&ctx.data_dir);

    match run(&cli, &reg, &events) {
        Ok(data) => emit(&envelope::ok(data), pretty),
        Err(e) => {
            reg.log_error(&e);
            emit(&envelope::err(&e), pretty);
            exit(1);
        }
    }
}
