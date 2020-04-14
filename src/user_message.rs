use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use textwrap;
use tui::{
    layout::{Constraint, Direction, Layout},
    widgets::{Paragraph, Text},
};

/// The max number of messages.
const NUM_MAX_MESSAGES: u16 = 5;
/// The max age of any message.
const MAX_MESSAGE_AGE: Duration = Duration::from_secs(10);

pub struct UserMessage {
    messages: Mutex<VecDeque<(Instant, String)>>,
}

impl UserMessage {
    pub fn new() -> Self {
        UserMessage {
            messages: Mutex::new(VecDeque::new()),
        }
    }

    /// Report a message to the user that will last for `MAX_MESSAGE_AGE`.
    ///
    /// Messages are pushed to a queue with a max size of `NUM_MAX_MESSAGES`.
    pub fn report(&self, message: &str) {
        let now = Instant::now();
        let message = message.to_string();
        let mut messages = self.messages.lock().unwrap();
        messages.push_back((now, message));
        if messages.len() >= NUM_MAX_MESSAGES as usize {
            messages.pop_front();
        }
    }

    /// Return a list of strings that represent messages to the user.
    fn get_lines(&self, max_age: Duration, max_width: u16) -> Vec<String> {
        let now = Instant::now();
        let messages = {
            let mut messages = self.messages.lock().unwrap();
            if let Some(oldest_allowed) = now.checked_sub(max_age) {
                loop {
                    if let Some((t, _)) = messages.front() {
                        if *t < oldest_allowed {
                            messages.pop_front();
                            continue;
                        }
                    }
                    break;
                }
            }
            messages
        };
        messages
            .iter()
            .flat_map(|(_, string)| textwrap::wrap_iter(string, max_width as usize))
            .map(|s| s.to_string())
            .collect()
    }

    /// Draw all user messages.
    pub fn draw<B>(
        &self,
        frame: &mut tui::terminal::Frame<B>,
        rect: tui::layout::Rect,
    ) -> tui::layout::Rect
    where
        B: tui::backend::Backend,
    {
        let lines = self.get_lines(MAX_MESSAGE_AGE, rect.width);
        if lines.is_empty() {
            rect
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints({
                    [
                        Constraint::Max(rect.height),
                        Constraint::Length(lines.len() as u16),
                    ]
                    .as_ref()
                })
                .split(rect);
            let (rect, message_rect) = (chunks[0], chunks[1]);

            let items: Vec<Text> = lines
                .iter()
                .map(|line| Text::raw(format!("{}\n", line)))
                .collect();
            let paragraph = Paragraph::new(items.iter()).wrap(true);
            frame.render_widget(paragraph, message_rect);

            rect
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tui::{backend::TestBackend, buffer::Buffer, Terminal};

    #[test]
    fn test_user_message() {
        let mut terminal = Terminal::new(TestBackend::new(50, 5)).unwrap();

        let message = UserMessage::new();
        message.report("This message will not be shown.");
        message.report("This is one message.");
        message.report("And here is a second.");
        message.report("This one is far too large to fit on one single line.");
        message.report("Another short message.");

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                message.draw(&mut frame, rect);
            })
            .unwrap();

        assert_eq!(
            *terminal.backend().buffer(),
            Buffer::with_lines(vec![
                "This is one message.                              ",
                "And here is a second.                             ",
                "This one is far too large to fit on one single    ",
                "line.                                             ",
                "Another short message.                            ",
            ])
        );
    }
}
