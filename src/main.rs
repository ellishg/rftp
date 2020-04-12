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

use events::{Event, EventListener};
use rftp::Rftp;
use std::error::Error;
use std::io::stdout;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;

fn main() {
    run().unwrap_or_else(|err| {
        eprintln!("Error: {}.", err);
        std::process::exit(1);
    });
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut rftp = Rftp::new()?;

    let mut terminal = {
        let stdout = stdout().into_raw_mode()?;
        let stdout = AlternateScreen::from(stdout);
        let backend = TermionBackend::new(stdout);
        tui::Terminal::new(backend)?
    };

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
            _ => (),
        }
    }

    Ok(())
}
