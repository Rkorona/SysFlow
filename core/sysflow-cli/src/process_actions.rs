//! `sysflow process stop|stop-all|oom|renice|sched|cpuset|freeze|unfreeze` — one-shot actions.
//!
//! The CLI only parses arguments; resolution, protection, identity re-checks and read-back all live
//! in `sysflow_modules::process::actions`, where they are covered by host tests.

use serde_json::Value;

use sysflow_core::error::{Error, Result};
use sysflow_core::registry::Registry;

use sysflow_modules::process::actions::{self, Target};

pub fn stop(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let (target, user) = target_and_user(rest, "stop")?;
    actions::stop(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, user)
}

pub fn oom(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "oom")?;
    let adjustment = parse_i64(args.get(1), "oom", "adjustment")?;
    actions::oom(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, adjustment)
}

pub fn renice(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "renice")?;
    let nice = parse_i64(args.get(1), "renice", "nice")?;
    actions::renice(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, nice)
}

pub fn sched(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "sched")?;
    let policy = args.get(1).ok_or_else(|| {
        Error::invalid_arg("process sched requires a policy (other|fifo|rr|batch|idle)")
    })?;
    let priority = args
        .get(2)
        .map(|value| parse_i64(Some(value), "sched", "priority"))
        .transpose()?;
    actions::sched(
        &reg.ctx().sysfs,
        &reg.ctx().data_dir,
        &target,
        policy,
        priority,
    )
}

pub fn cpuset(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), "cpuset")?;
    let group = args
        .get(1)
        .ok_or_else(|| Error::invalid_arg("process cpuset requires a group name"))?;
    actions::cpuset(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, group)
}

pub fn freeze(reg: &Registry, rest: &[&str], frozen: bool) -> Result<Value> {
    let args = positionals(rest);
    let target = parse_target(args.first(), if frozen { "freeze" } else { "unfreeze" })?;
    actions::freeze(&reg.ctx().sysfs, &reg.ctx().data_dir, &target, frozen)
}

fn target_and_user(rest: &[&str], verb: &str) -> Result<(Target, Option<u32>)> {
    let args = positionals(rest);
    let target = parse_target(args.first(), verb)?;
    let user =
        match flag(rest, "--user") {
            Some(value) => Some(value.parse::<u32>().map_err(|_| {
                Error::invalid_arg(format!("--user expects a number, got {value:?}"))
            })?),
            None => None,
        };
    Ok((target, user))
}

fn parse_target(value: Option<&&str>, verb: &str) -> Result<Target> {
    let value = value
        .ok_or_else(|| Error::invalid_arg(format!("process {verb} requires a PID or package")))?;
    actions::parse_target(value)
}

fn parse_i64(value: Option<&&str>, verb: &str, what: &str) -> Result<i64> {
    let value =
        value.ok_or_else(|| Error::invalid_arg(format!("process {verb} requires a {what}")))?;
    value
        .parse::<i64>()
        .map_err(|_| Error::invalid_arg(format!("{what} must be an integer, got {value:?}")))
}

/// Positional arguments, skipping `--flag value` pairs (only `--user` exists today).
fn positionals<'a>(rest: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in rest {
        if skip_next {
            skip_next = false;
            continue;
        }
        if *arg == "--user" {
            skip_next = true;
            continue;
        }
        if arg.starts_with("--") {
            continue;
        }
        out.push(*arg);
    }
    out
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let position = args.iter().position(|arg| *arg == name)?;
    args.get(position + 1).copied()
}

/// `process stop-all [--foreground]` — no target; the module selects the packages.
pub fn stop_all(reg: &Registry, rest: &[&str]) -> Result<Value> {
    let include_foreground = rest.contains(&"--foreground");
    actions::stop_all(&reg.ctx().sysfs, &reg.ctx().data_dir, include_foreground)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positionals_skip_options_and_their_values() {
        let args = ["com.example.app", "--user", "10", "--force"];
        assert_eq!(positionals(&args), vec!["com.example.app"]);
        let args = ["10123", "-5"];
        assert_eq!(positionals(&args), vec!["10123", "-5"]);
    }

    #[test]
    fn user_flag_is_read_next_to_the_target() {
        let args = ["com.example.app", "--user", "10"];
        assert_eq!(flag(&args, "--user"), Some("10"));
        assert_eq!(flag(&["com.example.app"], "--user"), None);
    }

    #[test]
    fn sched_positionals_accept_policy_and_optional_priority() {
        assert_eq!(
            positionals(&["10123", "fifo", "10"]),
            vec!["10123", "fifo", "10"]
        );
        assert_eq!(
            positionals(&["com.example.app", "other"]),
            vec!["com.example.app", "other"]
        );
    }
}
