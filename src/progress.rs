use crate::utils::{bitrate_to_string, bytes_to_string, duration_to_string};

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Gauge, Paragraph, Text},
};

/// The max age of any item in the history.
const HISTORY_MAX_AGE: Duration = Duration::from_secs(5);
const PROGRESSBAR_COLOR: Color = Color::LightBlue;

pub struct ProgressBars {
    file_progress_bars: Vec<Arc<ProgressFile>>,
    directory_progress_bars: Vec<Arc<ProgressDirectory>>,
}

impl ProgressBars {
    pub fn new() -> Self {
        ProgressBars {
            file_progress_bars: Vec::new(),
            directory_progress_bars: Vec::new(),
        }
    }

    pub fn push_directory_progress(&mut self, p: Arc<ProgressDirectory>) {
        self.directory_progress_bars.push(p);
    }

    pub fn push_file_progress(&mut self, p: Arc<ProgressFile>) {
        self.file_progress_bars.push(p);
    }

    pub fn is_empty(&self) -> bool {
        self.file_progress_bars.is_empty() && self.directory_progress_bars.is_empty()
    }

    pub fn retain_incomplete(&mut self) {
        self.file_progress_bars.retain(|p| !p.is_finished());
        self.directory_progress_bars.retain(|p| !p.is_finished());
    }

    pub fn draw<B>(
        &self,
        frame: &mut tui::terminal::Frame<B>,
        rect: tui::layout::Rect,
    ) -> tui::layout::Rect
    where
        B: tui::backend::Backend,
    {
        if self.is_empty() {
            rect
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Max(rect.height),
                        Constraint::Length(self.directory_progress_bars.len() as u16),
                        Constraint::Length(self.file_progress_bars.len() as u16),
                    ]
                    .as_ref(),
                )
                .split(rect);
            let (rect, directory_rect, file_rect) = (chunks[0], chunks[1], chunks[2]);

            let directory_rects = Layout::default()
                .constraints(
                    std::iter::repeat(Constraint::Length(1))
                        .take(self.directory_progress_bars.len())
                        .collect::<Vec<Constraint>>(),
                )
                .split(directory_rect);

            let file_rects = Layout::default()
                .constraints(
                    std::iter::repeat(Constraint::Length(1))
                        .take(self.file_progress_bars.len())
                        .collect::<Vec<Constraint>>(),
                )
                .split(file_rect);

            for (i, p) in self.directory_progress_bars.iter().enumerate() {
                p.draw(frame, directory_rects[i])
            }

            for (i, p) in self.file_progress_bars.iter().enumerate() {
                p.draw(frame, file_rects[i])
            }

            rect
        }
    }
}

pub struct ProgressDirectory {
    title: String,
    bytes_sent: AtomicU64,
    files_sent: AtomicU64,
    is_finished: AtomicBool,
}

impl ProgressDirectory {
    pub fn new(title: &str) -> Self {
        ProgressDirectory {
            title: title.to_string(),
            bytes_sent: AtomicU64::new(0),
            files_sent: AtomicU64::new(0),
            is_finished: AtomicBool::new(false),
        }
    }

    pub fn finish(&self) {
        self.is_finished.store(true, Ordering::Relaxed);
    }

    fn get_title(&self) -> &str {
        self.title.as_str()
    }

    pub fn is_finished(&self) -> bool {
        self.is_finished.load(Ordering::Relaxed)
    }

    pub fn inc(&self, file_size: u64) {
        self.bytes_sent.fetch_add(file_size, Ordering::Relaxed);
        self.files_sent.fetch_add(1, Ordering::Relaxed);
    }

    fn draw<B>(&self, frame: &mut tui::terminal::Frame<B>, rect: tui::layout::Rect)
    where
        B: tui::backend::Backend,
    {
        let files_sent = self.files_sent.load(Ordering::Relaxed);
        let info = format!(
            "{}  {} {}",
            bytes_to_string(self.bytes_sent.load(Ordering::Relaxed)),
            files_sent,
            if files_sent == 1 { "File" } else { "Files" }
        );
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

        let text = Text::styled(label, Style::default().fg(PROGRESSBAR_COLOR));

        frame.render_widget(Paragraph::new([text].iter()), rect);
    }
}

pub struct ProgressFile {
    title: String,
    bytes_sent: AtomicU64,
    total_bytes: u64,
    is_finished: AtomicBool,
    history: Mutex<VecDeque<(Instant, u64)>>,
}

impl ProgressFile {
    /// Create a thread-safe progress bar for a file with size `total_bytes` and a `title`.
    pub fn new(title: &str, total_bytes: u64) -> Self {
        let history = Mutex::new(vec![(Instant::now(), 0)].into_iter().collect());
        ProgressFile {
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

    /// Return `true` if this file has finished transfering.
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

    /// Draw this progress bar.
    fn draw<B>(&self, frame: &mut tui::terminal::Frame<B>, rect: tui::layout::Rect)
    where
        B: tui::backend::Backend,
    {
        let bitrate = self.get_current_bitrate();
        let eta = self
            .get_eta()
            .map(duration_to_string)
            .unwrap_or_else(|| "??:??".to_string());
        let info = format!(
            "{}/{}  {}  {} ETA",
            bytes_to_string(self.bytes_sent.load(Ordering::Relaxed)),
            bytes_to_string(self.total_bytes),
            bitrate_to_string(bitrate),
            eta
        );
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
        let gauge = Gauge::default()
            .style(Style::default().fg(PROGRESSBAR_COLOR))
            .label(&label)
            .ratio(self.get_ratio());

        frame.render_widget(gauge, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::buffer_without_style;
    use tui::{backend::TestBackend, buffer::Buffer, Terminal};

    #[test]
    fn test_draw_progress() {
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                let mut bars = ProgressBars::new();
                bars.push_file_progress(Arc::new(ProgressFile::new("just_started.txt", 100)));
                bars.push_file_progress(Arc::new(ProgressFile::new(
                    "this_is_a_really_long_filename.txt",
                    100,
                )));
                let finished = ProgressFile::new("finished.jpg", 100);
                finished.inc(50);
                finished.finish();
                bars.push_file_progress(Arc::new(finished));
                let dir = ProgressDirectory::new("/some/directory");
                dir.inc(1234);
                dir.inc(5678);
                bars.push_directory_progress(Arc::new(dir));
                let dir2 = ProgressDirectory::new("/some/other/directory");
                dir2.inc(9876);
                bars.push_directory_progress(Arc::new(dir2));
                let with_history = {
                    let now = Instant::now();
                    ProgressFile {
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
                bars.push_file_progress(Arc::new(with_history));
                bars.draw(&mut frame, rect);
            })
            .unwrap();

        assert_eq!(
            buffer_without_style(terminal.backend().buffer()),
            Buffer::with_lines(vec![
                "                                                            ",
                "                                                            ",
                "/some/directory                              6.9 KB  2 Files",
                "/some/other/directory                         9.9 KB  1 File",
                "just_started.txt               0 B/100 B  0 bit/s  ??:?? ETA",
                "this_is_a_really_long_filename 0 B/100 B  0 bit/s  ??:?? ETA",
                "finished.jpg                 100 B/100 B  0 bit/s  00:00 ETA",
                "with_history.dat         0 B/1.0 MB  131.1 Kbit/s  01:01 ETA",
            ])
        );
    }
}
