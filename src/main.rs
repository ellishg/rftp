#[macro_use]
extern crate crossbeam_channel;
#[macro_use]
extern crate clap;

mod connect;
mod events;
mod file;
mod progress;
mod rftp;
mod user_message;
mod utils;

use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use events::{Event, EventListener};
use rftp::Rftp;
use std::io::{stdout, Stdout, Write};
use tui::{backend::CrosstermBackend, Terminal};
use utils::Result;

fn main() {
    run_app().unwrap_or_else(|err| {
        eprintln!("Error: {}.", err);
        std::process::exit(1);
    });
}

/// Run the app.
fn run_app() -> Result<()> {
    let app = App::new()?;
    app.run()?;
    Ok(())
}

struct App {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    rftp: Rftp,
}

impl App {
    /// Create the `Rftp` and `Terminal` structs.
    pub fn new() -> Result<App> {
        let rftp = Rftp::new()?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        Ok(App { terminal, rftp })
    }

    /// Run the program.
    fn run(mut self) -> Result<()> {
        let rftp = &mut self.rftp;
        let terminal = &mut self.terminal;

        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(terminal.backend_mut(), EnterAlternateScreen)?;

        terminal.hide_cursor()?;

        let mut event_listener = EventListener::new(30.0);

        while rftp.is_alive() {
            match event_listener.get_next_event() {
                Ok(Event::Input(key)) => {
                    rftp.on_event(key)?;
                }
                Ok(Event::Tick) => {
                    rftp.tick()?;
                    terminal.draw(|frame| rftp.draw(frame))?;
                }
                Err(_) => (),
            }
        }

        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        crossterm::execute!(self.terminal.backend_mut(), LeaveAlternateScreen).unwrap();
        crossterm::terminal::disable_raw_mode().unwrap();
        self.terminal.show_cursor().unwrap();
    }
}
