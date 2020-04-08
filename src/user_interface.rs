use crate::progress::Progress;
use crate::rftp::Rftp;

use std::time::Duration;
use tui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Gauge, Paragraph, SelectableList, Text, Widget},
};

pub fn draw<B>(mut frame: tui::terminal::Frame<B>, rftp: &Rftp)
where
    B: tui::backend::Backend,
{
    let rect = frame.size();
    let rect = draw_user_message(&mut frame, rect, rftp.get_user_message());
    let rect = draw_progress_bars(&mut frame, rect, rftp.get_progress_bars());

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
        .split(rect);

    let (local_file_list_rect, remote_file_list_rect) = (chunks[0], chunks[1]);

    let (path, items, selected) = rftp.get_local_filelist_as_text();
    let title = format!("Local: {}", path);
    draw_file_list(&mut frame, local_file_list_rect, &title, items, selected);

    let (path, items, selected) = rftp.get_remote_filelist_as_text();
    let title = format!("Remote: {}", path);
    draw_file_list(&mut frame, remote_file_list_rect, &title, items, selected);
}

fn draw_user_message<B>(
    frame: &mut tui::terminal::Frame<B>,
    rect: tui::layout::Rect,
    message: Option<String>,
) -> tui::layout::Rect
where
    B: tui::backend::Backend,
{
    // TODO: Take a list of messages and give a timeout for each.
    // TODO: Messages that are too long should be split to different lines.
    match message {
        Some(message) => {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Max(rect.height), Constraint::Length(1)].as_ref())
                .split(rect);
            let (rect, message_rect) = (chunks[0], chunks[1]);
            Paragraph::new([Text::raw(message)].iter()).render(frame, message_rect);
            rect
        }
        None => rect,
    }
}

fn draw_file_list<B>(
    frame: &mut tui::terminal::Frame<B>,
    rect: tui::layout::Rect,
    title: &str,
    items: Vec<String>,
    selected: Option<usize>,
) where
    B: tui::backend::Backend,
{
    // TODO: Would like to style items by file type.
    //       https://github.com/fdehau/tui-rs/issues/254
    SelectableList::default()
        .block(Block::default().title(&title).borders(Borders::ALL))
        .items(items.as_slice())
        .select(selected)
        .highlight_style(Style::default().bg(Color::Yellow))
        .highlight_symbol(">>")
        .render(frame, rect);
}

fn draw_progress_bars<B>(
    frame: &mut tui::terminal::Frame<B>,
    rect: tui::layout::Rect,
    progress_bars: Vec<&Progress>,
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
            draw_progress_bar(frame, rects[i], p)
        }

        rect
    }
}

fn draw_progress_bar<B>(
    frame: &mut tui::terminal::Frame<B>,
    rect: tui::layout::Rect,
    progress: &Progress,
) where
    B: tui::backend::Backend,
{
    let bitrate = progress.get_current_bitrate();
    let eta = progress
        .get_eta()
        .map(|eta| duration_to_string(eta))
        .unwrap_or("??:??".to_string());
    let info = format!("{} {} ETA", bitrate_to_string(bitrate), eta);
    let width = frame.size().width as usize;
    let label = if info.len() + 5 >= width {
        format!(
            "{title:.max_width$}",
            title = progress.get_title(),
            max_width = width
        )
    } else {
        let width = width - info.len() - 1;
        format!(
            "{title:min_width$.max_width$} {info}",
            title = progress.get_title(),
            min_width = width,
            max_width = width,
            info = info,
        )
    };
    Gauge::default()
        .style(Style::default().fg(Color::Yellow))
        .label(&label)
        .ratio(progress.get_ratio())
        .render(frame, rect);
}

fn duration_to_string(t: Duration) -> String {
    let seconds = t.as_secs();
    let (seconds, minutes) = (seconds % 60, seconds / 60);
    let (minutes, hours) = (minutes % 60, minutes / 60);
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

fn bitrate_to_string(rate: u64) -> String {
    if rate < 1_000 {
        format!("{} bit/s", rate)
    } else if rate < 1_000_000 {
        format!("{:.1} Kbit/s", rate as f64 / 1e3)
    } else if rate < 1_000_000_000 {
        format!("{:.1} Mbit/s", rate as f64 / 1e6)
    } else {
        format!("{:.1} Gbit/s", rate as f64 / 1e9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
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
    fn test_user_message() {
        let mut terminal = Terminal::new(TestBackend::new(50, 3)).unwrap();

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                let rect = draw_user_message(&mut frame, rect, None);
                let rect =
                    draw_user_message(&mut frame, rect, Some("This is one message.".to_string()));
                let rect = draw_user_message(&mut frame, rect, None);
                let rect =
                    draw_user_message(&mut frame, rect, Some("And here is a second.".to_string()));
                let rect = draw_user_message(
                    &mut frame,
                    rect,
                    Some("This one is far too large to fit on one single line.".to_string()),
                );
                let _rect = draw_user_message(&mut frame, rect, None);
            })
            .unwrap();

        assert_eq!(
            *terminal.backend().buffer(),
            Buffer::with_lines(vec![
                "This one is far too large to fit on one single lin",
                "And here is a second.                             ",
                "This is one message.                              ",
            ])
        );
    }
}
