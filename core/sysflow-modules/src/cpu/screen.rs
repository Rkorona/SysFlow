//! Screen-off CPU profile: entering it, reconciling which cores may go offline, and restoring the
//! pre-offline state when the display wakes.
//!
//! Extracted from `cpu/mod.rs`, which had grown into frequency control, telemetry, validation and
//! this profile at once. The profile is the one part that keeps its own state
//! (`screen_saved`/`screen_active`) and must be restored exactly, so it reads better in isolation.
//! Functions stay `CpuModule` methods and are `pub(super)` because the parent's apply path calls them.

use super::*;

impl CpuModule {
    pub(super) fn begin_screen_off(&self, clusters: &[Cluster], st: &mut CpuState) {
        let mut ranges = BTreeMap::new();
        for c in clusters {
            ranges.insert(c.id.to_string(), (c.cur_min, c.cur_max));
        }
        let mut cores_off = Vec::new();
        let mut core_online_before = BTreeMap::new();
        // Capture the baseline of every controllable core, not only the currently selected
        // screen-off cores. This makes later edits to the selection reversible without guessing
        // from a live state that may already have been changed by this profile.
        for cpu in 1..self.cpu_count() {
            if !self.can_toggle(cpu) {
                continue;
            }
            let was_online = self.read_online(cpu) == 1;
            core_online_before.insert(cpu, was_online);
            if st.screen.cores.contains(&cpu) && was_online {
                cores_off.push(cpu);
            }
        }
        st.screen_saved = Some(state::ScreenSaved {
            ranges,
            cores_off,
            core_online_before,
            selected_cores: Some(st.screen.cores.clone()),
        });
        st.screen_active = true;
    }

    /// Reconcile edits to the screen-off core list while the profile is already active.
    /// Removed cores return to their normal online intent immediately; newly selected cores
    /// use the pre-profile baseline captured at screen-off entry.
    pub(super) fn reconcile_screen_cores(
        &self,
        st: &mut CpuState,
        online: &mut Option<Vec<i64>>,
        desired_online: Option<&Value>,
    ) {
        if !st.screen_active {
            return;
        }
        let Some(saved) = st.screen_saved.as_mut() else {
            return;
        };

        // Old state files did not store the selected list explicitly. Their map/list contained
        // only cores that were turned off by the previous implementation.
        let previous: BTreeSet<usize> = saved
            .selected_cores
            .clone()
            .map(|v| v.into_iter().collect())
            .unwrap_or_else(|| {
                if !saved.core_online_before.is_empty() {
                    saved.core_online_before.keys().copied().collect()
                } else {
                    saved.cores_off.iter().copied().collect()
                }
            });
        let selected: BTreeSet<usize> = st.screen.cores.iter().copied().collect();
        let desired_base = desired_online.and_then(Value::as_array).map(|values| {
            (0..self.cpu_count())
                .map(|cpu| {
                    values
                        .get(cpu)
                        .and_then(Value::as_i64)
                        .unwrap_or_else(|| self.read_online(cpu))
                })
                .collect::<Vec<_>>()
        });
        let mut want = online
            .take()
            .unwrap_or_else(|| (0..self.cpu_count()).map(|c| self.read_online(c)).collect());
        let mut changed = false;

        // Restore the normal state of cores removed from the screen-off selection.
        for cpu in previous.difference(&selected).copied().collect::<Vec<_>>() {
            if !self.can_toggle(cpu) {
                continue;
            }
            let base = desired_base
                .as_ref()
                .and_then(|v| v.get(cpu).copied())
                .or_else(|| {
                    saved
                        .core_online_before
                        .get(&cpu)
                        .map(|v| if *v { 1 } else { 0 })
                })
                .or_else(|| saved.cores_off.contains(&cpu).then_some(1))
                .unwrap_or_else(|| self.read_online(cpu));
            saved.core_online_before.insert(cpu, base != 0);
            if let Some(slot) = want.get_mut(cpu) {
                *slot = if base != 0 { 1 } else { 0 };
                changed = true;
            }
        }

        // Keep the baseline current for each selected core. If the persisted normal cpu.online
        // list was explicitly edited while the display was off, that user intent takes priority.
        for cpu in selected.iter().copied() {
            if !self.can_toggle(cpu) {
                continue;
            }
            let base = desired_base
                .as_ref()
                .and_then(|v| v.get(cpu).copied())
                .or_else(|| {
                    saved
                        .core_online_before
                        .get(&cpu)
                        .map(|v| if *v { 1 } else { 0 })
                })
                .unwrap_or_else(|| self.read_online(cpu));
            saved.core_online_before.insert(cpu, base != 0);
        }
        saved.selected_cores = Some(selected.iter().copied().collect());
        saved.cores_off = selected
            .iter()
            .filter_map(|cpu| {
                saved
                    .core_online_before
                    .get(cpu)
                    .copied()
                    .filter(|was_online| *was_online)
                    .map(|_| *cpu)
            })
            .collect();

        if changed {
            *online = Some(want);
        }
    }

    pub(super) fn end_screen_off(
        &self,
        st: &mut CpuState,
        online: &mut Option<Vec<i64>>,
        desired_online: Option<&Value>,
    ) -> Result<()> {
        if let Err(e) = self.end_screen_off_inner(st, online, desired_online) {
            self.emit_runtime_event(
                NewSystemEvent::new(
                    "cpu",
                    "power",
                    "screen_off_restore_failed",
                    "error",
                    "CPU screen-off restore failed",
                )
                .message(format!(
                    "Failed to restore normal CPU limits/cores after screen-off: {}",
                    e.msg
                ))
                .metadata(json!({ "error_code": e.code.as_str() }))
                .dedupe("cpu.screen_off.restore_failed", 300),
            );
            return Err(e);
        }
        Ok(())
    }

    pub(super) fn end_screen_off_inner(
        &self,
        st: &mut CpuState,
        online: &mut Option<Vec<i64>>,
        desired_online: Option<&Value>,
    ) -> Result<()> {
        let Some(saved) = st.screen_saved.clone() else {
            st.screen_active = false;
            return Ok(());
        };

        // Work out the final normal online state before touching policy paths. A policy may
        // disappear when its last CPU is shut down, so cores that should be online must be
        // brought back first (the old SoCFlow restore order did the same).
        let mut want = online
            .take()
            .unwrap_or_else(|| (0..self.cpu_count()).map(|c| self.read_online(c)).collect());
        let restore_cores: BTreeSet<usize> = saved
            .selected_cores
            .clone()
            .map(|v| v.into_iter().collect())
            .unwrap_or_else(|| {
                if !saved.core_online_before.is_empty() {
                    saved.core_online_before.keys().copied().collect()
                } else {
                    saved.cores_off.iter().copied().collect()
                }
            });
        let desired_base = desired_online.and_then(Value::as_array);
        for cpu in restore_cores {
            if !self.can_toggle(cpu) {
                continue;
            }
            let desired = desired_base
                .and_then(|values| values.get(cpu).and_then(Value::as_i64))
                .map(|value| value != 0)
                .or_else(|| saved.core_online_before.get(&cpu).copied())
                .or_else(|| saved.cores_off.contains(&cpu).then_some(true));
            if let (Some(slot), Some(desired)) = (want.get_mut(cpu), desired) {
                *slot = if desired { 1 } else { 0 };
            }
        }

        // Stage only online transitions now. Do not take any core down until frequency ranges
        // have been restored; the final `online` plan is applied by apply_inner afterwards.
        let mut bring_up: Vec<i64> = (0..self.cpu_count())
            .map(|cpu| self.read_online(cpu))
            .collect();
        for (cpu, desired) in want.iter().enumerate() {
            if *desired != 0 {
                if let Some(slot) = bring_up.get_mut(cpu) {
                    *slot = 1;
                }
            }
        }
        let mut brought_up = Vec::new();
        self.apply_online(&bring_up, &mut brought_up)?;

        // Keep the active marker and saved snapshot intact until every restore write succeeds.
        // The screen coordinator retries failed transitions, and the staged online cores keep
        // the cpufreq policy nodes present for that retry.
        for (id, (min, max)) in &saved.ranges {
            let Ok(policy) = id.parse::<u32>() else {
                continue;
            };
            let min_path = Self::policy_path(policy, "scaling_min_freq");
            let max_path = Self::policy_path(policy, "scaling_max_freq");
            let current_min = self.fs.read_i64(&min_path).unwrap_or(*min);
            if *max < current_min {
                self.fs.write_str(&min_path, &min.to_string())?;
                self.fs.write_str(&max_path, &max.to_string())?;
            } else {
                self.fs.write_str(&max_path, &max.to_string())?;
                self.fs.write_str(&min_path, &min.to_string())?;
            }
        }

        // Preserve an explicit cpu.online request even when the saved screen-off core list is
        // empty (or contains only locked cores). apply_inner applies this final plan after ranges.
        *online = Some(want);
        st.screen_saved = None;
        st.screen_active = false;
        Ok(())
    }

    pub(super) fn screen_online(
        &self,
        st: &CpuState,
        online: Option<Vec<i64>>,
    ) -> Option<Vec<i64>> {
        if !st.screen_active || st.screen.cores.is_empty() {
            return online;
        }
        let mut want = match online {
            Some(w) => w,
            None => (0..self.cpu_count()).map(|c| self.read_online(c)).collect(),
        };
        for c in &st.screen.cores {
            if let Some(slot) = want.get_mut(*c) {
                if self.can_toggle(*c) {
                    *slot = 0;
                }
            }
        }
        Some(want)
    }

    // ---- apply --------------------------------------------------------------

    pub(super) fn validate_screen_request(&self, req: &Map<String, Value>) -> Result<()> {
        if let Some(caps) = req.get(ITEM_SCREEN_MAX).and_then(Value::as_object) {
            let clusters = self.clusters();
            for (key, value) in caps {
                let id = key.parse::<u32>().map_err(|_| {
                    Error::invalid_arg(format!("{ITEM_SCREEN_MAX}: bad cluster id {key:?}"))
                })?;
                let cluster = clusters
                    .iter()
                    .find(|cluster| cluster.id == id)
                    .ok_or_else(|| Error::invalid_arg(format!("policy{id} does not exist")))?;
                let cap = value.as_i64().ok_or_else(|| {
                    Error::invalid_arg(format!("{ITEM_SCREEN_MAX}[{id}]: expected kHz integer"))
                })?;
                if cap < 0 {
                    return Err(Error::invalid_arg(format!(
                        "{ITEM_SCREEN_MAX}[{id}]: must be 0 (unlimited) or positive"
                    )));
                }
                if cap > 0 {
                    let low = cluster
                        .hw_min
                        .or_else(|| cluster.freqs.first().copied())
                        .unwrap_or(0);
                    let high = cluster
                        .hw_max
                        .or_else(|| cluster.freqs.last().copied())
                        .unwrap_or(i64::MAX);
                    if (low > 0 && cap < low) || (high > 0 && cap > high) {
                        return Err(Error::invalid_arg(format!(
                            "{ITEM_SCREEN_MAX}[{id}]: {cap} kHz is outside hardware range {low}–{high} kHz"
                        )));
                    }
                }
            }
        }

        if let Some(Value::Array(cores)) = req.get(ITEM_SCREEN_CORES) {
            for (cpu, value) in cores.iter().enumerate() {
                if value.as_i64() == Some(1) && (cpu == 0 || !self.can_toggle(cpu)) {
                    return Err(Error::invalid_arg(format!(
                        "{ITEM_SCREEN_CORES}: cpu{cpu} cannot be turned off"
                    )));
                }
            }
        }
        Ok(())
    }
}
