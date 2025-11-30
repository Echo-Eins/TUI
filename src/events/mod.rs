use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use tokio::time::{interval, Duration, Interval};

pub enum AppEvent {
    Input(Event),
    Tick,
}

pub struct EventHandler {
    event_stream: EventStream,
    tick_interval: Interval,
}

impl EventHandler {
    pub fn new(tick_rate_ms: u64) -> Self {
        Self {
            event_stream: EventStream::new(),
            tick_interval: interval(Duration::from_millis(tick_rate_ms)),
        }
    }

    pub async fn next(&mut self) -> AppEvent {
        tokio::select! {
            _ = self.tick_interval.tick() => AppEvent::Tick,
            event = self.event_stream.next() => {
                match event {
                    Some(Ok(evt)) => AppEvent::Input(evt),
                    _ => AppEvent::Tick, // Fallback to tick on error or None
                }
            }
        }
    }
}
