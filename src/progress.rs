use crate::utils::{bitrate_to_string, duration_to_string};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Gauge, Widget},
};

const HISTORY_MAX_AGE: Duration = Duration::from_secs(5);

pub struct Progress {
    title: String,
    bytes_sent: AtomicU64,
    total_bytes: u64,
    is_finished: AtomicBool,
    history: Mutex<VecDeque<(Instant, u64)>>,
}

impl Progress {
    /// Create a thread-safe progress bar with `total_bytes` and a `title`.
    pub fn new(title: &str, total_bytes: u64) -> Self {
        let history = Mutex::new(vec![(Instant::now(), 0)].into_iter().collect());
        Progress {
            title: title.to_string(),
            bytes_sent: AtomicU64::new(0),
            total_bytes,
            is_finished: AtomicBool::new(false),
            history,
        }
    }

    /// Return the title of this progress bar.
    pub fn get_title(&self) -> &str {
        self.title.as_str()
    }

    /// Return the fraction that this progress bar has completed.
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

    /// Return `true` if this progress bar is finished.
    pub fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Relaxed)
    }

    /// Tell the progress bar how many `bytes` have been successfully sent.
    pub fn inc(&self, bytes: u64) {
        let now = Instant::now();
        {
            let mut history = self.history.lock().unwrap();
            history.push_back((now, bytes));
            if let Some(max_age) = now.checked_sub(HISTORY_MAX_AGE) {
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
        }
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Finish this progress bar.
    pub fn finish(&self) {
        self.bytes_sent.store(self.total_bytes, Ordering::Relaxed);
        self.history.lock().unwrap().clear();
        self.is_finished.store(true, Ordering::Relaxed);
    }

    /// Return the current estimated number of bits sent per second.
    ///
    /// This value is computed over the last 5 seconds.
    pub fn get_current_bitrate(&self) -> u64 {
        let history = self.history.lock().unwrap();
        let instants = history.iter().map(|(t, _)| t);
        let bits_sent = 8 * history.iter().map(|(_, b)| b).sum::<u64>();
        let seconds = instants
            .clone()
            .min()
            .and_then(|oldest| {
                instants
                    .max()
                    .map(|yongest| yongest.duration_since(*oldest).as_secs_f64())
            })
            .unwrap_or(0.0);
        if seconds == 0.0 {
            0
        } else {
            (bits_sent as f64 / seconds) as u64
        }
    }

    /// Return the estimated time before progress is completed using `self.get_current_bitrate()`.
    pub fn get_eta(&self) -> Option<Duration> {
        if self.is_finished() {
            Some(Duration::from_secs(0))
        } else {
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

    /// Draw a list of progress bars.
    pub fn draw_progress_bars<B>(
        progress_bars: Vec<&Progress>,
        frame: &mut tui::terminal::Frame<B>,
        rect: tui::layout::Rect,
    ) -> tui::layout::Rect
    where
        B: tui::backend::Backend,
    {
        if progress_bars.is_empty() {
            rect
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Max(rect.height),
                        Constraint::Length(progress_bars.len() as u16),
                    ]
                    .as_ref(),
                )
                .split(rect);
            let (rect, progress_rect) = (chunks[0], chunks[1]);

            let rects = Layout::default()
                .constraints(
                    std::iter::repeat(Constraint::Length(1))
                        .take(progress_bars.len())
                        .collect::<Vec<Constraint>>(),
                )
                .split(progress_rect);

            for (i, p) in progress_bars.iter().enumerate() {
                p.draw(frame, rects[i])
            }

            rect
        }
    }

    /// Draw this progress bar.
    fn draw<B>(&self, frame: &mut tui::terminal::Frame<B>, rect: tui::layout::Rect)
    where
        B: tui::backend::Backend,
    {
        let bitrate = self.get_current_bitrate();
        let eta = self
            .get_eta()
            .map(|eta| duration_to_string(eta))
            .unwrap_or("??:??".to_string());
        let info = format!("{} {} ETA", bitrate_to_string(bitrate), eta);
        let width = frame.size().width as usize;
        let label = if info.len() + 5 >= width {
            format!(
                "{title:.max_width$}",
                title = self.get_title(),
                max_width = width
            )
        } else {
            let width = width - info.len() - 1;
            format!(
                "{title:min_width$.max_width$} {info}",
                title = self.get_title(),
                min_width = width,
                max_width = width,
                info = info,
            )
        };
        Gauge::default()
            .style(Style::default().fg(Color::Yellow))
            .label(&label)
            .ratio(self.get_ratio())
            .render(frame, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::*;
    use tui::{backend::TestBackend, buffer::Buffer, Terminal};

    #[test]
    fn test_duration() {
        assert_eq!(
            duration_to_string(Duration::from_secs(12)),
            "00:12".to_string()
        );
        assert_eq!(
            duration_to_string(Duration::from_secs(123)),
            "02:03".to_string()
        );
        assert_eq!(
            duration_to_string(Duration::from_secs(2 * 60 * 60 + 40 * 60 + 59)),
            "2:40:59".to_string()
        );
    }

    #[test]
    fn test_bitrate() {
        assert_eq!(bitrate_to_string(4), "4 bit/s".to_string());
        assert_eq!(bitrate_to_string(1e3 as u64), "1.0 Kbit/s".to_string());
        assert_eq!(bitrate_to_string(3e6 as u64), "3.0 Mbit/s".to_string());
        assert_eq!(bitrate_to_string(7e9 as u64), "7.0 Gbit/s".to_string());
    }

    #[test]
    fn test_draw_progress() {
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                let just_started = Progress::new("just_started.txt", 100);
                let long = Progress::new("this_is_a_really_long_filename.txt", 100);
                let finished = Progress::new("finished.jpg", 100);
                finished.inc(50);
                finished.finish();
                let with_history = {
                    let now = Instant::now();
                    Progress {
                        title: "with_history.dat".into(),
                        bytes_sent: AtomicU64::new(0),
                        total_bytes: 1e6 as u64,
                        is_finished: AtomicBool::new(false),
                        history: Mutex::new(
                            vec![
                                (now, 0),
                                (now + Duration::from_secs_f32(0.25), 4 * 1024),
                                (now + Duration::from_secs_f32(0.5), 4 * 1024),
                                (now + Duration::from_secs_f32(0.75), 4 * 1024),
                                (now + Duration::from_secs_f32(1.0), 4 * 1024),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    }
                };
                Progress::draw_progress_bars(
                    vec![&just_started, &long, &with_history, &finished],
                    &mut frame,
                    rect,
                );
            })
            .unwrap();

        assert_eq!(
            buffer_without_style(terminal.backend().buffer()),
            Buffer::with_lines(vec![
                "                                                  ",
                "                                                  ",
                "                                                  ",
                "                                                  ",
                "just_started.txt                 0 bit/s ??:?? ETA",
                "this_is_a_really_long_filename.t 0 bit/s ??:?? ETA",
                "with_history.dat            131.1 Kbit/s 01:01 ETA",
                "finished.jpg                     0 bit/s 00:00 ETA",
            ])
        );
    }
}
