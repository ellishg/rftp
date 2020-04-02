use std::time::Duration;
use termion::event::Key;
use termion::input::TermRead;
use tokio::sync::mpsc::Receiver;

pub enum Event {
    Input(Key),
    Tick,
}

pub struct EventListener {
    key_receiver: Receiver<Key>,
    tick_receiver: Receiver<()>,
}

impl EventListener {
    pub fn new(ticks_per_second: f64) -> Self {
        let key_receiver = {
            let (mut tx, rx) = tokio::sync::mpsc::channel(1024);
            tokio::spawn(async move {
                for key in std::io::stdin().keys() {
                    if let Ok(key) = key {
                        if let Err(_) = tx.send(key).await {
                            return;
                        }
                    } else {
                        return;
                    }
                }
            });
            rx
        };

        let tick_receiver = {
            let (mut tx, rx) = tokio::sync::mpsc::channel(128);
            tokio::spawn(async move {
                let duration = Duration::from_secs_f64(1.0 / ticks_per_second);
                let mut interval = tokio::time::interval(duration);
                loop {
                    interval.tick().await;
                    if let Err(_) = tx.send(()).await {
                        return;
                    }
                }
            });
            rx
        };

        EventListener {
            key_receiver,
            tick_receiver,
        }
    }

    pub async fn get_next_event(&mut self) -> Event {
        tokio::select! {
            key = self.key_receiver.recv() => {
                Event::Input(key.unwrap())
            },
            _ = self.tick_receiver.recv() => {
                Event::Tick
            }
        }
    }
}
