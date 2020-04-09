use crate::progress::Progress;
use crate::rftp::Rftp;

use tui::{
    layout::{Constraint, Direction, Layout},
    widgets::{Paragraph, Text, Widget},
};

pub fn draw<B>(mut frame: tui::terminal::Frame<B>, rftp: &Rftp)
where
    B: tui::backend::Backend,
{
    let rect = frame.size();
    let rect = draw_user_message(&mut frame, rect, rftp.get_user_message());
    let rect = Progress::draw_progress_bars(rftp.get_progress_bars(), &mut frame, rect);
    rftp.get_file_list().draw(&mut frame, rect);
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
}
