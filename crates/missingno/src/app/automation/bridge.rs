//! Carrying an automation call from a socket thread into the app's `update`
//! loop. This mirrors the session event bridge, inverted: there the app
//! receives events, here it receives calls. One app-lifetime subscription
//! ([`automation_calls_worker`]) hands the app an Iced [`UnboundedSender`] as
//! its first item; the endpoint's client threads push each call through it, so
//! the call runs on the UI thread where the widget tree lives.

use std::sync::{Arc, Mutex};

use iced::futures::channel::mpsc::UnboundedSender;
use iced::futures::{Stream, StreamExt, stream};

/// One automation call awaiting an answer: the tool name, its arguments, and a
/// blocking channel the UI thread posts the JSON result back on.
#[derive(Debug, Clone)]
pub struct AutomationCall {
    pub tool: String,
    pub args: serde_json::Value,
    pub reply: std::sync::mpsc::Sender<serde_json::Value>,
}

/// Items the app-lifetime subscription yields: the sink handed over once, then
/// every call forwarded through it.
#[derive(Debug, Clone)]
pub enum AutomationBridge {
    /// The first item: the sink the endpoint pushes calls into.
    Ready(UnboundedSender<AutomationCall>),
    /// A call forwarded from a socket client thread.
    Call(AutomationCall),
}

/// A slot the endpoint reads the call sink from. The sink arrives on the
/// subscription's first item, which may be after the endpoint opens, so the
/// endpoint holds this shared slot rather than the sink directly.
#[derive(Clone, Default)]
pub struct SharedSink(Arc<Mutex<Option<UnboundedSender<AutomationCall>>>>);

impl SharedSink {
    pub fn set(&self, sink: UnboundedSender<AutomationCall>) {
        *self.0.lock().unwrap() = Some(sink);
    }

    pub fn get(&self) -> Option<UnboundedSender<AutomationCall>> {
        self.0.lock().unwrap().clone()
    }
}

/// The app-lifetime subscription worker. A non-capturing `fn` (required by
/// `Subscription::run`): it creates an Iced channel, yields its sender first,
/// then streams every call pushed into it.
pub fn automation_calls_worker() -> impl Stream<Item = AutomationBridge> {
    let (sink, calls) = iced::futures::channel::mpsc::unbounded::<AutomationCall>();
    stream::once(async move { AutomationBridge::Ready(sink) })
        .chain(calls.map(AutomationBridge::Call))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor::block_on;

    // The first item hands over the sink; a call pushed through it arrives as
    // the next stream item — the drive path, no polling.
    #[test]
    fn ready_first_then_forwarded_calls() {
        let mut stream = Box::pin(automation_calls_worker());
        let sink = match block_on(stream.next()) {
            Some(AutomationBridge::Ready(sink)) => sink,
            other => panic!("first item must be Ready, got {other:?}"),
        };
        let (reply, _rx) = std::sync::mpsc::channel();
        sink.unbounded_send(AutomationCall {
            tool: "status".into(),
            args: serde_json::json!({}),
            reply,
        })
        .unwrap();
        assert!(matches!(
            block_on(stream.next()),
            Some(AutomationBridge::Call(call)) if call.tool == "status"
        ));
    }

    #[test]
    fn shared_sink_hands_over_late() {
        let shared = SharedSink::default();
        assert!(shared.get().is_none());
        let (sink, _calls) = iced::futures::channel::mpsc::unbounded::<AutomationCall>();
        shared.set(sink);
        assert!(shared.get().is_some());
    }
}
