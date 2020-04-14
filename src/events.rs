use crossbeam_channel::{tick, unbounded, Receiver, RecvError};
use crossterm;
use std::thread;
use std::time::{Duration, Instant};

pub enum Event {
    Input(crossterm::event::KeyEvent),
    Tick,
}

pub struct EventListener {
    key_receiver: Receiver<crossterm::event::KeyEvent>,
    tick_receiver: Receiver<Instant>,
}

impl EventListener {
    pub fn new(ticks_per_second: f64) -> Self {
        let key_receiver = {
            let (tx, rx) = unbounded();
            thread::spawn(move || loop {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(event)) => {
                        tx.send(event).unwrap();
                    }
                    Ok(_) => {}
                    Err(_) => {}
                }
            });
            rx
        };

        let tick_receiver = tick(Duration::from_secs_f64(1.0 / ticks_per_second));

        EventListener {
            key_receiver,
            tick_receiver,
        }
    }

    /// Return the next "tick" or key press that occures.
    pub fn get_next_event(&mut self) -> Result<Event, RecvError> {
        select! {
            recv(self.key_receiver) -> key => Ok(Event::Input(key?)),
            recv(self.tick_receiver) -> instant => {
                let _ = instant?;
                Ok(Event::Tick)
            },
        }
    }
}
