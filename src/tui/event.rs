use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::worker::WorkerEvent;

#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Tick,
    Worker(WorkerEvent),
}

pub fn spawn_crossterm_reader(tx: mpsc::UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        loop {
            if event::poll(Duration::from_millis(66)).unwrap_or(false) {
                if let Ok(CrosstermEvent::Key(key)) = event::read() {
                    if tx.send(AppEvent::Key(key)).is_err() {
                        break;
                    }
                }
            } else if tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });
}
