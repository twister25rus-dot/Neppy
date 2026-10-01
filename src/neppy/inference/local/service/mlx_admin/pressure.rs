//! System and per-process memory readings, and the pressure state machine.
//!
//! Sampling is sudo-free and in-process. On macOS every reading is a
//! `sysctlbyname` or `proc_pid_rusage` call, microseconds each, so the
//! watchdog can sample every two seconds while a request runs without
//! measurable cost. Linux reads `/proc/meminfo` and `/proc/pressure/memory`;
//! anything else falls back to `sysinfo`.
//!
//! [`PressureTracker::observe`] is pure — it takes a reading and a clock value
//! and returns a state — so the hysteresis is table-tested without a machine
//! under pressure.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::neppy::config::schema::MlxWorkerConfig;

/// Swap growth since the worker loaded that counts as pressure on its own.
pub(crate) const SWAP_GROWTH_LIMIT_BYTES: u64 = 512 * 1024 * 1024;

const GIB: u64 = 1024 * 1024 * 1024;
/// Floor and ceiling of the derived load reserve.
const MIN_RESERVE_BYTES: u64 = 2 * GIB;
const MAX_RESERVE_BYTES: u64 = 8 * GIB;
/// Share of physical memory the derived load reserve starts from, percent.
const RESERVE_PCT: u64 = 15;
/// What an explicitly requested start (`mlx.start`, autostart) must leave
/// available once the model is loaded, whatever the reserve says.
pub(crate) const EXPLICIT_START_FLOOR_BYTES: u64 = 2 * GIB;
/// Percentage points above `elevated_avail_pct` that availability must reach
/// to recover when `recover_avail_pct` is left at `0` (automatic).
pub(crate) const AUTO_RECOVER_MARGIN_PCT: f64 = 5.0;

/// One system-wide memory reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub(crate) struct SystemMemory {
    pub(crate) total_bytes: u64,
    /// Available memory as the kernel estimates it, percent of total.
    pub(crate) avail_pct: f64,
    pub(crate) avail_bytes: u64,
    /// Kernel pressure level: 1 normal, 2 warn, 4 critical. Platforms without
    /// one report 1 unless another signal says otherwise.
    pub(crate) pressure_level: u8,
    pub(crate) swap_used_bytes: u64,
    pub(crate) compressed_bytes: u64,
}

/// One process's memory reading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub(crate) struct ProcMem {
    /// `phys_footprint` on macOS — the number Activity Monitor shows, which
    /// counts Metal buffers that RSS misses. RSS elsewhere.
    pub(crate) footprint_bytes: u64,
    pub(crate) rss_bytes: u64,
}

/// Memory pressure as the worker policy sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PressureState {
    Normal,
    Elevated,
    Critical,
}

impl PressureState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Elevated => "elevated",
            Self::Critical => "critical",
        }
    }
}

/// A state change, with the signal that caused it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Transition {
    pub(crate) from: PressureState,
    pub(crate) to: PressureState,
    pub(crate) reason: String,
}

/// Hysteresis over [`SystemMemory`] readings.
#[derive(Debug, Clone)]
pub(crate) struct PressureTracker {
    state: PressureState,
    /// When the recovery condition started holding continuously.
    recovering_since: Option<Instant>,
    /// Swap in use when the worker loaded; growth past it is a signal.
    swap_baseline: Option<u64>,
}

impl Default for PressureTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl PressureTracker {
    pub(crate) fn new() -> Self {
        Self {
            state: PressureState::Normal,
            recovering_since: None,
            swap_baseline: None,
        }
    }

    pub(crate) fn state(&self) -> PressureState {
        self.state
    }

    /// Record swap in use at worker load (`Some`) or clear it at stop (`None`).
    pub(crate) fn set_swap_baseline(&mut self, swap_used_bytes: Option<u64>) {
        self.swap_baseline = swap_used_bytes;
    }

    /// Fold one reading into the state.
    ///
    /// - Critical: level 4, or available below `critical_avail_pct`.
    /// - Elevated: level ≥ 2, available below `elevated_avail_pct`, the worker
    ///   over `budget_bytes`, or swap grown past 512 MiB since load.
    /// - Back to Normal only after level 1, available ≥ [`recover_threshold_pct`]
    ///   and no worker/swap signal have held for `recover_hold_secs`.
    ///   Critical steps down to Elevated as soon as the critical signal clears.
    pub(crate) fn observe(
        &mut self,
        sample: &SystemMemory,
        worker_footprint: Option<u64>,
        budget_bytes: u64,
        now: Instant,
        cfg: &MlxWorkerConfig,
    ) -> (PressureState, Option<Transition>) {
        let (raw, reason) = self.raw_state(sample, worker_footprint, budget_bytes, cfg);
        let previous = self.state;

        let next = if raw >= previous {
            // Rising (or steady) pressure is acted on immediately.
            self.recovering_since = None;
            raw
        } else {
            let recovered = raw == PressureState::Normal
                && sample.pressure_level <= 1
                && sample.avail_pct >= recover_threshold_pct(cfg);
            if recovered {
                let since = *self.recovering_since.get_or_insert(now);
                if now.saturating_duration_since(since)
                    >= Duration::from_secs(cfg.recover_hold_secs)
                {
                    self.recovering_since = None;
                    PressureState::Normal
                } else {
                    previous.min(PressureState::Elevated)
                }
            } else {
                self.recovering_since = None;
                // Critical cleared but not recovered: still no new work.
                previous.min(PressureState::Elevated).max(raw)
            }
        };

        self.state = next;
        let transition = (next != previous).then(|| Transition {
            from: previous,
            to: next,
            reason: if next < previous {
                if next == PressureState::Normal {
                    format!(
                        "recovered: level=1 avail={:.1}% held {}s",
                        sample.avail_pct, cfg.recover_hold_secs
                    )
                } else {
                    format!("critical signal cleared: {reason}")
                }
            } else {
                reason
            },
        });
        (next, transition)
    }

    fn raw_state(
        &self,
        sample: &SystemMemory,
        worker_footprint: Option<u64>,
        budget_bytes: u64,
        cfg: &MlxWorkerConfig,
    ) -> (PressureState, String) {
        if sample.pressure_level >= 4 {
            return (
                PressureState::Critical,
                "kernel pressure level 4".to_string(),
            );
        }
        if sample.avail_pct < cfg.critical_avail_pct {
            return (
                PressureState::Critical,
                format!(
                    "available {:.1}% < {:.1}%",
                    sample.avail_pct, cfg.critical_avail_pct
                ),
            );
        }
        if sample.pressure_level >= 2 {
            return (
                PressureState::Elevated,
                format!("kernel pressure level {}", sample.pressure_level),
            );
        }
        if sample.avail_pct < cfg.elevated_avail_pct {
            return (
                PressureState::Elevated,
                format!(
                    "available {:.1}% < {:.1}%",
                    sample.avail_pct, cfg.elevated_avail_pct
                ),
            );
        }
        if let Some(footprint) = worker_footprint {
            if budget_bytes > 0 && footprint > budget_bytes {
                return (
                    PressureState::Elevated,
                    format!(
                        "worker footprint {} MiB > budget {} MiB",
                        footprint / MIB,
                        budget_bytes / MIB
                    ),
                );
            }
        }
        if let Some(baseline) = self.swap_baseline {
            let grown = sample.swap_used_bytes.saturating_sub(baseline);
            if grown > SWAP_GROWTH_LIMIT_BYTES {
                return (
                    PressureState::Elevated,
                    format!("swap grew {} MiB since load", grown / MIB),
                );
            }
        }
        (PressureState::Normal, "no pressure signal".to_string())
    }
}

const MIB: u64 = 1024 * 1024;

/// Memory that must stay available after a managed (lazy) load: the configured
/// reserve, or `clamp(15% of total, 2 GiB, 8 GiB)`.
///
/// The derived reserve used to be `max(8 GiB, 25% of total)`, which on a 16 or
/// 24 GiB Mac is half the machine or more: nearly every model was refused, even
/// ones that fit comfortably. 15% scales with the machine (2.4 / 3.6 / 5.4 /
/// 8.0 GiB at 16 / 24 / 36 / 64 GiB), the floor keeps a small machine from
/// reserving nothing, and the ceiling stops a big one from reserving more than
/// macOS and a few apps need.
pub(crate) fn reserve_bytes(cfg: &MlxWorkerConfig, total_bytes: u64) -> u64 {
    if cfg.reserve_gib > 0.0 {
        return (cfg.reserve_gib * GIB as f64) as u64;
    }
    (total_bytes.saturating_mul(RESERVE_PCT) / 100).clamp(MIN_RESERVE_BYTES, MAX_RESERVE_BYTES)
}

/// Available memory, percent, that must hold before pressure returns to
/// Normal. `recover_avail_pct = 0` means `elevated_avail_pct + 5`, so recovery
/// is always reachable from wherever Elevated starts; an explicit value is
/// honoured but never taken below the elevated threshold, since recovering
/// below the level that raised the alarm would flap.
pub(crate) fn recover_threshold_pct(cfg: &MlxWorkerConfig) -> f64 {
    if cfg.recover_avail_pct > 0.0 {
        cfg.recover_avail_pct.max(cfg.elevated_avail_pct)
    } else {
        cfg.elevated_avail_pct + AUTO_RECOVER_MARGIN_PCT
    }
}

/// Sample system-wide memory.
pub(crate) fn sample_system() -> SystemMemory {
    let sample = imp::sample_system().unwrap_or_else(sysinfo_fallback);
    log::trace!(
        "[mlx:worker] system memory avail={:.1}% level={} swap={}MiB",
        sample.avail_pct,
        sample.pressure_level,
        sample.swap_used_bytes / MIB
    );
    sample
}

/// Sample one process. `None` when it does not exist or cannot be read.
pub(crate) fn sample_process(pid: u32) -> Option<ProcMem> {
    imp::sample_process(pid).or_else(|| {
        let target = sysinfo::Pid::from_u32(pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
        system.process(target).map(|process| ProcMem {
            footprint_bytes: process.memory(),
            rss_bytes: process.memory(),
        })
    })
}

fn sysinfo_fallback() -> SystemMemory {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let total = system.total_memory();
    let avail = system.available_memory();
    SystemMemory {
        total_bytes: total,
        avail_pct: percent(avail, total),
        avail_bytes: avail,
        pressure_level: 1,
        swap_used_bytes: system.used_swap(),
        compressed_bytes: 0,
    }
}

pub(crate) fn percent(part: u64, total: u64) -> f64 {
    if total == 0 {
        return 100.0;
    }
    part as f64 * 100.0 / total as f64
}

/// Parse `ioreg -c IOAccelerator` output for the `"In use system memory"`
/// counter. The `(driver)` variant is a different counter and is skipped.
pub(crate) fn parse_gpu_in_use(ioreg: &str) -> Option<u64> {
    const KEY: &str = "\"In use system memory\"=";
    let start = ioreg.find(KEY)? + KEY.len();
    let digits: String = ioreg[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// GPU memory in use, from `ioreg` (macOS only; no sudo). Spawns a process, so
/// callers rate-limit it.
pub(crate) fn gpu_in_use_bytes() -> Option<u64> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("ioreg")
        .args(["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"])
        .output()
        .ok()?;
    if !output.status.success() {
        log::debug!("[mlx:worker] ioreg exited with {}", output.status);
        return None;
    }
    parse_gpu_in_use(&String::from_utf8_lossy(&output.stdout))
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
/// Parse `/proc/meminfo` into `(total, available, swap_used)` bytes.
pub(crate) fn parse_meminfo(text: &str) -> Option<(u64, u64, u64)> {
    let field = |name: &str| -> Option<u64> {
        text.lines()
            .find(|line| line.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse::<u64>()
            .ok()
            .map(|kib| kib * 1024)
    };
    let total = field("MemTotal:")?;
    let avail = field("MemAvailable:")?;
    let swap_used = field("SwapTotal:")
        .unwrap_or(0)
        .saturating_sub(field("SwapFree:").unwrap_or(0));
    Some((total, avail, swap_used))
}

#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
/// Map `/proc/pressure/memory` to a macOS-style level: `full avg10 ≥ 10` is 4,
/// `some avg10 ≥ 10` is 2, otherwise 1.
pub(crate) fn parse_psi_level(text: &str) -> u8 {
    let avg10 = |kind: &str| -> f64 {
        text.lines()
            .find(|line| line.starts_with(kind))
            .and_then(|line| {
                line.split_whitespace()
                    .find_map(|part| part.strip_prefix("avg10="))
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or(0.0)
    };
    if avg10("full") >= 10.0 {
        4
    } else if avg10("some") >= 10.0 {
        2
    } else {
        1
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CString;
    use std::mem::{size_of, MaybeUninit};

    use super::{percent, ProcMem, SystemMemory};

    /// Read a fixed-size sysctl value by name.
    fn sysctl<T: Copy>(name: &str) -> Option<T> {
        let cname = CString::new(name).ok()?;
        let mut value = MaybeUninit::<T>::uninit();
        let mut len = size_of::<T>();
        // SAFETY: `value` is writable storage of `len` bytes and `cname` is a
        // valid NUL-terminated string; the kernel writes at most `len` bytes.
        let rc = unsafe {
            libc::sysctlbyname(
                cname.as_ptr(),
                value.as_mut_ptr().cast::<libc::c_void>(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || len != size_of::<T>() {
            return None;
        }
        // SAFETY: the call succeeded and filled exactly `size_of::<T>()` bytes.
        Some(unsafe { value.assume_init() })
    }

    pub(super) fn sample_system() -> Option<SystemMemory> {
        let total: u64 = sysctl("hw.memsize")?;
        let level_pct: i32 = sysctl("kern.memorystatus_level")?;
        let pressure: i32 = sysctl("kern.memorystatus_vm_pressure_level").unwrap_or(1);
        let swap: Option<libc::xsw_usage> = sysctl("vm.swapusage");
        let compressed: u64 = sysctl("vm.compressor_bytes_used").unwrap_or(0);
        let avail_pct = f64::from(level_pct.clamp(0, 100));
        let avail_bytes = (total as f64 * avail_pct / 100.0) as u64;
        Some(SystemMemory {
            total_bytes: total,
            avail_pct: percent(avail_bytes, total),
            avail_bytes,
            pressure_level: u8::try_from(pressure.clamp(1, 4)).unwrap_or(1),
            swap_used_bytes: swap.map(|s| s.xsu_used).unwrap_or(0),
            compressed_bytes: compressed,
        })
    }

    pub(super) fn sample_process(pid: u32) -> Option<ProcMem> {
        let pid = libc::c_int::try_from(pid).ok()?;
        let mut usage = MaybeUninit::<libc::rusage_info_v4>::uninit();
        // SAFETY: `usage` is writable storage of the exact size for
        // `RUSAGE_INFO_V4`; the kernel initializes it on success (return 0).
        let rc = unsafe {
            libc::proc_pid_rusage(
                pid,
                libc::RUSAGE_INFO_V4,
                usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
            )
        };
        if rc != 0 {
            return None;
        }
        // SAFETY: `proc_pid_rusage` returned success and initialized `usage`.
        let usage = unsafe { usage.assume_init() };
        Some(ProcMem {
            footprint_bytes: usage.ri_phys_footprint,
            rss_bytes: usage.ri_resident_size,
        })
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{parse_meminfo, parse_psi_level, percent, ProcMem, SystemMemory};

    pub(super) fn sample_system() -> Option<SystemMemory> {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let (total, avail, swap_used) = parse_meminfo(&meminfo)?;
        let level = std::fs::read_to_string("/proc/pressure/memory")
            .map(|text| parse_psi_level(&text))
            .unwrap_or(1);
        Some(SystemMemory {
            total_bytes: total,
            avail_pct: percent(avail, total),
            avail_bytes: avail,
            pressure_level: level,
            swap_used_bytes: swap_used,
            compressed_bytes: 0,
        })
    }

    pub(super) fn sample_process(_pid: u32) -> Option<ProcMem> {
        // sysinfo's RSS is the best cheap number here; the caller falls back.
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::{ProcMem, SystemMemory};

    pub(super) fn sample_system() -> Option<SystemMemory> {
        None
    }

    pub(super) fn sample_process(_pid: u32) -> Option<ProcMem> {
        None
    }
}

#[cfg(test)]
#[path = "pressure_tests.rs"]
mod tests;
