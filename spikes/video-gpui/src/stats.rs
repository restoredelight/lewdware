//! What the spike measures about itself. Linux-only (procfs); other platforms print zeros.
//!
//! * CPU: `getrusage(RUSAGE_SELF)` user+system time over wall time, as % of one core.
//! * Memory: RSS from `/proc/self/statm`, and GPU memory from the DRM fdinfo of the process
//!   (`drm-total-*` / `drm-resident-*`, summed over memory regions and deduplicated by
//!   `drm-client-id`). On Intel iGPUs textures live in system memory but are *not* in RSS, so the
//!   fdinfo number is the one that shows atlas growth.
//! * Frames: for every frame adopted for display, its pts and the reference clock (audio position,
//!   or the wall clock on the clock-master path) at the moment it was handed to gpui.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub fn cpu_time() -> Duration {
    unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
        tv(usage.ru_utime) + tv(usage.ru_stime)
    }
}

pub fn rss_mb() -> f64 {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0.0;
    };
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    (pages * page) as f64 / (1024.0 * 1024.0)
}

/// Sum of `drm-total-*` over this process's DRM clients, in MiB. Each client (one per open
/// render-node fd, deduplicated by client id) reports its own buffer objects.
pub fn gpu_mb() -> f64 {
    let Ok(dir) = std::fs::read_dir("/proc/self/fdinfo") else {
        return 0.0;
    };
    let mut by_client: HashMap<String, f64> = HashMap::new();
    for entry in dir.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if !text.contains("drm-driver") {
            continue;
        }
        let mut client = String::new();
        let mut total_kib = 0.0;
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            if key == "drm-client-id" {
                client = value.to_string();
            } else if key.starts_with("drm-total-") {
                let mut parts = value.split_whitespace();
                let n: f64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let unit = parts.next().unwrap_or("");
                total_kib += match unit {
                    "MiB" => n * 1024.0,
                    "KiB" => n,
                    "GiB" => n * 1024.0 * 1024.0,
                    _ => n / 1024.0,
                };
            }
        }
        by_client.insert(client, total_kib);
    }
    by_client.values().sum::<f64>() / 1024.0
}

pub struct FrameRecord {
    pub sequence: u64,
    /// Reference clock minus pts when the frame was adopted: positive is late.
    pub lateness_ms: f64,
}

pub struct Stats {
    pub started: Instant,
    pub cpu_at_start: Duration,
    pub rss_start: f64,
    pub gpu_start: f64,
    pub rss_max: f64,
    pub gpu_max: f64,
    pub frames: Vec<FrameRecord>,
    /// Frames the UI took from the decoder but replaced before they were drawn (timer pump only).
    pub superseded: u32,
    /// Time spent in the view's `render` (UI thread), summed.
    pub render_time: Duration,
    pub renders: u32,
    /// Time spent in the video element's `paint` (atlas insert + upload staging), summed.
    pub paint_time: Duration,
    pub paints: u32,
    last_tick: Instant,
    last_tick_cpu: Duration,
}

impl Stats {
    pub fn new() -> Self {
        let now = Instant::now();
        let cpu = cpu_time();
        let (rss, gpu) = (rss_mb(), gpu_mb());
        Self {
            started: now,
            cpu_at_start: cpu,
            rss_start: rss,
            gpu_start: gpu,
            rss_max: rss,
            gpu_max: gpu,
            frames: Vec::new(),
            superseded: 0,
            render_time: Duration::ZERO,
            renders: 0,
            paint_time: Duration::ZERO,
            paints: 0,
            last_tick: now,
            last_tick_cpu: cpu,
        }
    }

    /// Start (or restart) the measured window: everything before it is warm-up.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// CPU % since the previous tick, plus current RSS/GPU. Updates the maxima.
    pub fn tick(&mut self) -> (f64, f64, f64) {
        let now = Instant::now();
        let cpu = cpu_time();
        let pct =
            (cpu - self.last_tick_cpu).as_secs_f64() / (now - self.last_tick).as_secs_f64() * 100.0;
        self.last_tick = now;
        self.last_tick_cpu = cpu;
        let (rss, gpu) = (rss_mb(), gpu_mb());
        self.rss_max = self.rss_max.max(rss);
        self.gpu_max = self.gpu_max.max(gpu);
        (pct, rss, gpu)
    }

    pub fn cpu_pct_total(&self) -> f64 {
        (cpu_time() - self.cpu_at_start).as_secs_f64() / self.started.elapsed().as_secs_f64()
            * 100.0
    }

    /// Converted frames missing between adopted frames. Sequence numbers continue over loops
    /// and reset on seeking. A long variable-rate frame is not mistaken for dropped frames.
    pub fn gaps(&self) -> u64 {
        self.frames
            .windows(2)
            .map(|w| {
                w[1].sequence
                    .saturating_sub(w[0].sequence)
                    .saturating_sub(1)
            })
            .sum()
    }

    pub fn lateness_percentiles(&self) -> (f64, f64, f64, f64) {
        if self.frames.is_empty() {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let mut l: Vec<f64> = self.frames.iter().map(|f| f.lateness_ms).collect();
        l.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p = |q: f64| l[((l.len() - 1) as f64 * q).round() as usize];
        (p(0.0), p(0.5), p(0.95), p(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_count_missing_frames_and_ignore_decoder_resets() {
        let mut stats = Stats::new();
        stats.frames = [10, 11, 15, 0, 1, 2]
            .into_iter()
            .map(|sequence| FrameRecord {
                sequence,
                lateness_ms: 0.0,
            })
            .collect();
        assert_eq!(stats.gaps(), 3);
    }
}
