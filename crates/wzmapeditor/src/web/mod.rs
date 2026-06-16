//! Shared browser primitives for the web (wasm32) build.

use std::sync::mpsc::{Receiver, TryRecvError};

pub(crate) mod cache;
pub(crate) mod dom;

/// Outcome of draining a per-frame channel with [`drain_once`].
pub(crate) enum Drain<T> {
    /// Nothing arrived this frame; the async producer is still working.
    Pending,
    /// A value arrived; the receiver has been cleared.
    Ready(T),
    /// The sender dropped without producing a value; the receiver is cleared.
    Closed,
}

/// Take at most one value from an optional per-frame channel.
///
/// Clears `rx` once a value arrives or the sender drops, so a single poll site
/// owns the channel's whole lifecycle. With `keep_awake`, an empty channel
/// requests a repaint so an in-flight async producer keeps the frame loop
/// running while it works.
pub(crate) fn drain_once<T>(
    rx: &mut Option<Receiver<T>>,
    ctx: &egui::Context,
    keep_awake: bool,
) -> Drain<T> {
    let Some(receiver) = rx.as_ref() else {
        return Drain::Pending;
    };
    match receiver.try_recv() {
        Ok(value) => {
            *rx = None;
            Drain::Ready(value)
        }
        Err(TryRecvError::Empty) => {
            if keep_awake {
                ctx.request_repaint();
            }
            Drain::Pending
        }
        Err(TryRecvError::Disconnected) => {
            *rx = None;
            Drain::Closed
        }
    }
}
