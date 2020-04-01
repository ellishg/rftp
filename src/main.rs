#[macro_use]
extern crate clap;

mod events;
mod file;
mod progress;
mod rftp;
mod user_interface;

use events::{Event, EventListener};
use rftp::Rftp;
use std::error::Error;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;

#[tokio::main]
async fn main() {
    run().await.unwrap_or_else(|err| {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    });
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut rftp = Rftp::new()?;

    let mut terminal = {
        let stdout = std::io::stdout().into_raw_mode()?;
        let stdout = AlternateScreen::from(stdout);
        let backend = TermionBackend::new(stdout);
        tui::Terminal::new(backend)?
    };

    let mut event_listener = EventListener::new(30.0);

    while rftp.is_alive() {
        match event_listener.get_next_event().await {
            Event::Input(key) => {
                rftp.on_event(key)?;
            }
            Event::Tick => {
                rftp.tick()?;
                terminal.draw(|frame| user_interface::draw(frame, &rftp))?;
            }
        }
    }

    Ok(())
}
