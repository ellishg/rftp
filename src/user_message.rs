use std::collections::VecDeque;
use std::sync::Mutex;
use textwrap;
use tokio::time::{Duration, Instant};
use tui::{
    layout::{Constraint, Direction, Layout},
    widgets::{Paragraph, Text, Widget},
};

const NUM_MAX_MESSAGES: u16 = 5;
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

    pub fn report(&self, message: &str) {
        let now = Instant::now();
        let message = message.to_string();
        self.messages.lock().unwrap().push_back((now, message));
    }

    fn get_lines(&self, max_age: Duration, max_width: u16) -> Vec<String> {
        let now = Instant::now();
        let messages = {
            let mut messages = self.messages.lock().unwrap();
            messages.truncate(NUM_MAX_MESSAGES as usize);
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
            Paragraph::new(items.iter())
                .wrap(true)
                .render(frame, message_rect);

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
        message.report("This is one message.");
        message.report("And here is a second.");
        message.report("This one is far too large to fit on one single line.");

        terminal
            .draw(|mut frame| {
                let rect = frame.size();
                message.draw(&mut frame, rect);
            })
            .unwrap();

        assert_eq!(
            *terminal.backend().buffer(),
            Buffer::with_lines(vec![
                "                                                  ",
                "This is one message.                              ",
                "And here is a second.                             ",
                "This one is far too large to fit on one single    ",
                "line.                                             ",
            ])
        );
    }
}
