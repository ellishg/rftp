use crossbeam_channel::{tick, unbounded, Receiver, RecvError};
use std::io::stdin;
use std::thread;
use std::time::{Duration, Instant};
use termion::event::Key;
use termion::input::TermRead;

pub enum Event {
    Input(Key),
    Tick,
}

pub struct EventListener {
    key_receiver: Receiver<Key>,
    tick_receiver: Receiver<Instant>,
}

impl EventListener {
    pub fn new(ticks_per_second: f64) -> Self {
        let key_receiver = {
            let (tx, rx) = unbounded();
            thread::spawn(move || {
                for key in stdin().keys() {
                    if let Ok(key) = key {
                        tx.send(key).unwrap();
                    } else {
                        return;
                    }
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
