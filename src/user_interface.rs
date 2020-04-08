use crate::progress::Progress;
use crate::rftp::Rftp;

use tui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph, SelectableList, Text, Widget},
};

pub fn draw<B>(mut frame: tui::terminal::Frame<B>, rftp: &Rftp)
where
    B: tui::backend::Backend,
{
    let rect = frame.size();
    let rect = draw_user_message(&mut frame, rect, rftp.get_user_message());
    let rect = Progress::draw_progress_bars(rftp.get_progress_bars(), &mut frame, rect);

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
    // TODO: Move to file.rs
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

#[cfg(test)]
mod tests {
    use super::*;
    use tui::{backend::TestBackend, buffer::Buffer, Terminal};

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

    #[test]
    fn test_file_list() {
        let mut terminal = Terminal::new(TestBackend::new(50, 8)).unwrap();

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                // TODO: Take a `FileList` instead.
                draw_file_list(
                    &mut frame,
                    rect,
                    "The Title",
                    vec![
                        "myfile.txt".to_string(),
                        "myotherfile.dat".to_string(),
                        "mydirectory/".to_string(),
                    ],
                    None,
                );
            })
            .unwrap();

        assert_eq!(
            *terminal.backend().buffer(),
            Buffer::with_lines(vec![
                "┌The Title───────────────────────────────────────┐",
                "│myfile.txt                                      │",
                "│myotherfile.dat                                 │",
                "│mydirectory/                                    │",
                "│                                                │",
                "│                                                │",
                "│                                                │",
                "└────────────────────────────────────────────────┘",
            ])
        );
    }
}
