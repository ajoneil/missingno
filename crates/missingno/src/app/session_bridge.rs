//! Bridging the shared session's event stream into an Iced subscription.
//!
//! The session component pushes [`SessionEvent`]s onto a `std::sync::mpsc`
//! channel per subscriber — a channel Iced's async runtime cannot poll. One
//! app-lifetime subscription ([`session_events_worker`]) hands the app an Iced
//! [`UnboundedSender`] as its first item; when a game loads, the app spawns a
//! per-game bridge thread that blocks on the session's receiver and forwards
//! each event through that sender, so redraws arrive promptly with no polling.

use iced::futures::channel::mpsc::UnboundedSender;
use iced::futures::{Stream, StreamExt, stream};

use missingno_session::{SessionEvent, SessionHandle};

/// Items the app-lifetime subscription yields: the sender handed over once at
/// startup, then every session event forwarded through it.
#[derive(Debug, Clone)]
pub enum SessionBridge {
    /// The first item: the sink a per-game bridge thread forwards events into.
    Ready(UnboundedSender<SessionEvent>),
    /// A session event, forwarded from a per-game bridge thread.
    Event(SessionEvent),
}

/// The app-lifetime subscription worker. A non-capturing `fn` (required by
/// `Subscription::run`): it creates an Iced channel, yields its sender as the
/// first item, then streams every event forwarded into it. It spawns no thread
/// itself — the per-game bridge thread ([`spawn_bridge`]) does the forwarding.
pub fn session_events_worker() -> impl Stream<Item = SessionBridge> {
    let (sink, events) = iced::futures::channel::mpsc::unbounded::<SessionEvent>();
    stream::once(async move { SessionBridge::Ready(sink) }).chain(events.map(SessionBridge::Event))
}

/// Spawn a per-game bridge thread: it owns a fresh subscription to `handle`'s
/// events and forwards each into `sink` until the session drops (the engine
/// thread ends, its subscriber senders drop, and `recv` errs). No polling — the
/// blocking `recv` wakes on each event.
pub fn spawn_bridge(handle: &SessionHandle, sink: UnboundedSender<SessionEvent>) {
    let events = handle.subscribe();
    std::thread::Builder::new()
        .name("session-bridge".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sink.unbounded_send(event).is_err() {
                    break;
                }
            }
        })
        .expect("spawn session bridge thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor::block_on;

    // The first item hands over the sink; a subsequent event forwarded through
    // it arrives as the next stream item — the redraw path, no polling.
    #[test]
    fn ready_first_then_forwarded_events() {
        let mut stream = Box::pin(session_events_worker());
        let sink = match block_on(stream.next()) {
            Some(SessionBridge::Ready(sink)) => sink,
            other => panic!("first item must be Ready, got {other:?}"),
        };
        sink.unbounded_send(SessionEvent::FrameReady).unwrap();
        sink.unbounded_send(SessionEvent::Stopped).unwrap();
        assert!(matches!(
            block_on(stream.next()),
            Some(SessionBridge::Event(SessionEvent::FrameReady))
        ));
        assert!(matches!(
            block_on(stream.next()),
            Some(SessionBridge::Event(SessionEvent::Stopped))
        ));
    }
}
