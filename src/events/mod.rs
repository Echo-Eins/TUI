use crossterm::event::{Event, EventStream};
use futures::StreamExt;

pub struct EventHandler {
    event_stream: EventStream,
}

impl EventHandler {
    pub fn new() -> Self {
        Self {
            event_stream: EventStream::new(),
        }
    }

    pub async fn next(&mut self) -> Option<Event> {
        self.event_stream.next().await.and_then(|e| e.ok())
    }
}
