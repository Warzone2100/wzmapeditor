//! Unified task and progress abstractions for background work.
//!
//! [`TaskHandle::spawn`] centralizes the `mpsc::channel + thread::spawn +
//! poll-receiver-in-update` recipe so the startup pipeline, asset
//! loaders, and panel workers don't each reinvent it. Lifecycle
//! ("spawned" / "ready") is logged once per task at debug level.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc;

/// A handle to a background task.
///
/// Wraps a one-shot receiver delivering the worker's result. The worker
/// thread is named after `label` so it shows up in profilers and panic
/// backtraces.
///
/// Progress reporting is separate: pass an `Arc<AtomicU32>` to
/// [`Self::spawn_with_progress`] and read it directly. Keeping progress
/// off the handle lets callers store the arc next to other UI state
/// without cloning per frame.
pub struct TaskHandle<T> {
    receiver: mpsc::Receiver<T>,
    label: &'static str,
}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + 'static> TaskHandle<T> {
    /// Spawn `work` on a fresh background thread; the handle carries its
    /// result back to the polling thread. Dropping the handle before the
    /// worker finishes wastes the work but is not racy: `tx.send`
    /// silently fails once the receiver is gone.
    pub fn spawn<F>(label: &'static str, work: F) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name(label.to_string())
            .spawn(move || {
                let _ = tx.send(work());
            })
            .expect("spawn task thread");
        log::debug!("[task:{label}] spawned");
        Self {
            receiver: rx,
            label,
        }
    }

    /// Spawn `work` with an atomic progress counter the worker writes
    /// into (thousandths 0..=1000 by convention). The same `Arc` is
    /// passed into the closure so callers can keep the original next to
    /// other UI state for direct polling.
    pub fn spawn_with_progress<F>(label: &'static str, progress: Arc<AtomicU32>, work: F) -> Self
    where
        F: FnOnce(Arc<AtomicU32>) -> T + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name(label.to_string())
            .spawn(move || {
                let _ = tx.send(work(progress));
            })
            .expect("spawn task thread");
        log::debug!("[task:{label}] spawned (with progress)");
        Self {
            receiver: rx,
            label,
        }
    }
}

// wasm32 has no usable OS threads in the browser (GitHub Pages cannot send the
// COOP/COEP headers SharedArrayBuffer requires), so each task runs synchronously
// at spawn time and queues its result for the same `try_take` poll the native
// path uses. Callers are unchanged; the `Send` bound is dropped since nothing
// crosses a thread boundary.
#[cfg(target_arch = "wasm32")]
impl<T: 'static> TaskHandle<T> {
    pub fn spawn<F>(label: &'static str, work: F) -> Self
    where
        F: FnOnce() -> T + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(work());
        log::debug!("[task:{label}] ran inline (wasm)");
        Self {
            receiver: rx,
            label,
        }
    }

    pub fn spawn_with_progress<F>(label: &'static str, progress: Arc<AtomicU32>, work: F) -> Self
    where
        F: FnOnce(Arc<AtomicU32>) -> T + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let _ = tx.send(work(progress));
        log::debug!("[task:{label}] ran inline with progress (wasm)");
        Self {
            receiver: rx,
            label,
        }
    }
}

impl<T> TaskHandle<T> {
    /// Non-blocking poll. Returns `Some(result)` exactly once, when the
    /// worker has finished. Intended call site is `update()`, once per
    /// frame. Logs a debug line on the frame the result is observed.
    pub fn try_take(&self) -> Option<T> {
        self.receiver.try_recv().ok().inspect(|_| {
            log::debug!("[task:{}] ready", self.label);
        })
    }
}

impl<T> std::fmt::Debug for TaskHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskHandle")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    /// Block until `f` returns `Some`, retrying for up to `max`. Avoids
    /// flaky fixed-duration sleeps.
    fn poll_until<T, F: FnMut() -> Option<T>>(mut f: F, max: Duration) -> T {
        let deadline = Instant::now() + max;
        loop {
            if let Some(v) = f() {
                return v;
            }
            assert!(Instant::now() < deadline, "task did not complete in time");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn spawn_round_trips_a_value() {
        let task = TaskHandle::spawn("test_round_trip", || 42_u32);
        let v = poll_until(|| task.try_take(), Duration::from_secs(2));
        assert_eq!(v, 42);
    }

    #[test]
    fn try_take_returns_none_until_ready_then_some_then_none() {
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let task = TaskHandle::spawn("test_gated", move || {
            // Block until the test confirms it observed pending state.
            let _ = gate_rx.recv();
            "done"
        });
        assert!(task.try_take().is_none(), "result before signal");
        gate_tx.send(()).expect("signal worker");
        let v = poll_until(|| task.try_take(), Duration::from_secs(2));
        assert_eq!(v, "done");
        assert!(
            task.try_take().is_none(),
            "second take after consumption returns None"
        );
    }

    #[test]
    fn spawn_with_progress_exposes_counter() {
        let progress = Arc::new(AtomicU32::new(0));
        let task = TaskHandle::spawn_with_progress("test_progress", progress.clone(), |p| {
            p.store(500, Ordering::Relaxed);
            p.store(1000, Ordering::Relaxed);
            "ok"
        });
        let v = poll_until(|| task.try_take(), Duration::from_secs(2));
        assert_eq!(v, "ok");
        assert_eq!(progress.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn dropping_handle_does_not_panic_worker() {
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let task = TaskHandle::spawn("test_dropped_rx", move || {
            let _ = gate_rx.recv();
            42
        });
        drop(task);
        // After the receiver drops, the worker's `tx.send` returns `Err`;
        // the helper swallows it with `let _`, so the thread doesn't panic.
        gate_tx.send(()).expect("signal worker");
        std::thread::sleep(Duration::from_millis(20));
    }
}
