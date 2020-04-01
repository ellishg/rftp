use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Progress {
    title: String,
    bytes_sent: AtomicU64,
    total_bytes: u64,
    start_time: Instant,
    is_finished: AtomicBool,
}

impl Progress {
    pub fn new(title: String, total_bytes: u64) -> Self {
        Progress {
            title,
            bytes_sent: AtomicU64::new(0),
            total_bytes,
            start_time: Instant::now(),
            is_finished: AtomicBool::new(false),
        }
    }

    pub fn get_title(&self) -> &str {
        self.title.as_str()
    }

    pub fn get_ratio(&self) -> f64 {
        let bytes_sent = self.bytes_sent.load(Ordering::Relaxed);
        (bytes_sent as f64) / (self.total_bytes as f64)
    }

    pub fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Relaxed)
    }

    pub fn inc(&self, bytes: u64) {
        let bytes_sent = self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
        assert!(bytes_sent <= self.total_bytes);
    }

    pub fn finish(&self) {
        self.is_finished.store(true, Ordering::Relaxed);
    }

    pub fn get_current_bitrate(&self) -> u64 {
        // TODO: Use moving average
        let bytes_sent = self.bytes_sent.load(Ordering::Relaxed) as f64;
        let seconds = {
            let runtime = Instant::now() - self.start_time;
            runtime.as_secs() as f64 + runtime.subsec_nanos() as f64 * 1e-9
        };
        8 * (bytes_sent / seconds) as u64
    }

    pub fn get_eta(&self) -> Option<Duration> {
        let bytes_per_second = self.get_current_bitrate() / 8;
        if bytes_per_second == 0 {
            None
        } else {
            let remaining_bytes = self.total_bytes - self.bytes_sent.load(Ordering::Relaxed);
            Some(Duration::from_secs_f64(
                remaining_bytes as f64 / bytes_per_second as f64,
            ))
        }
    }
}
