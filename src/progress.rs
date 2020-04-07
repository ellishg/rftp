use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const HISTORY_MAX_AGE: Duration = Duration::from_secs(5);

pub struct Progress {
    title: String,
    bytes_sent: AtomicU64,
    total_bytes: u64,
    is_finished: AtomicBool,
    history: Mutex<VecDeque<(Instant, u64)>>,
}

impl Progress {
    pub fn new(title: String, total_bytes: u64) -> Self {
        let history = Mutex::new(vec![(Instant::now(), 0)].into_iter().collect());
        Progress {
            title,
            bytes_sent: AtomicU64::new(0),
            total_bytes,
            is_finished: AtomicBool::new(false),
            history,
        }
    }

    pub fn get_title(&self) -> &str {
        self.title.as_str()
    }

    pub fn get_ratio(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            let bytes_sent = self.bytes_sent.load(Ordering::Relaxed);
            if bytes_sent >= self.total_bytes {
                1.0
            } else {
                (bytes_sent as f64) / (self.total_bytes as f64)
            }
        }
    }

    pub fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Relaxed)
    }

    pub fn inc(&self, bytes: u64) {
        {
            let now = Instant::now();
            let max_age = now - HISTORY_MAX_AGE;
            let mut history = self.history.lock().unwrap();
            history.push_back((now, bytes));
            loop {
                if let Some((t, _)) = history.front() {
                    if *t <= max_age {
                        history.pop_front();
                        continue;
                    }
                }
                break;
            }
        }
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn finish(&self) {
        self.is_finished.store(true, Ordering::Relaxed);
    }

    pub fn get_current_bitrate(&self) -> u64 {
        let history = self.history.lock().unwrap();
        if let Some(age) = history.iter().map(|(t, _)| t).min() {
            let bits_sent = 8.0 * history.iter().map(|(_, b)| b).sum::<u64>() as f64;
            let seconds = {
                let runtime = Instant::now() - *age;
                runtime.as_secs() as f64 + runtime.subsec_nanos() as f64 * 1e-9
            };
            if seconds == 0.0 {
                0
            } else {
                (bits_sent / seconds) as u64
            }
        } else {
            0
        }
    }

    pub fn get_eta(&self) -> Option<Duration> {
        let bytes_per_second = self.get_current_bitrate() / 8;
        if bytes_per_second == 0 {
            None
        } else {
            let bytes_sent = self.bytes_sent.load(Ordering::Relaxed);
            if bytes_sent <= self.total_bytes {
                let remaining_bytes = self.total_bytes - bytes_sent;
                Some(Duration::from_secs_f64(
                    remaining_bytes as f64 / bytes_per_second as f64,
                ))
            } else {
                None
            }
        }
    }
}
