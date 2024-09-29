use core::cell::RefCell;
use core::future::Future;
use core::sync::atomic::{AtomicU16, Ordering};
use core::task::{Context, Poll, Waker};

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::blocking_mutex::Mutex;

#[derive(Debug)]
enum StateInner {
    Waiting(Waker),
    Signaled,
}

struct State<T, const WAKERS: usize> {
    value: Option<T>,
    waiters: heapless::Vec<(u16, StateInner), WAKERS>,
}

impl<T, const WAKERS: usize> State<T, WAKERS> {
    const fn new() -> Self {
        Self {
            value: None,
            waiters: heapless::Vec::new(),
        }
    }
}

/// Single-slot signaling primitive that retains the value after being read.
///
/// This is similar to a [`Signal`](embassy_sync::signal::Signal), but it does not clear the inner value
/// when it is read. This is useful when the receiver needs to read the latest value multiple times.
///
/// StickySignals are generally declared as `static`s and then borrowed as required.
///
/// ```
/// use embassy_sync::signal::StickySignal;
/// use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
///
/// enum SomeCommand {
///   On,
///   Off,
/// }
///
/// static SOME_STICKY_SIGNAL: StickySignal<CriticalSectionRawMutex, SomeCommand> = StickySignal::new();
/// # or, if you don't need to share the signal between threads
/// static SINGLE_THREAD_STICKY_SIGNAL: StaticCell<StickySignal<NoopRawMutex, SomeCommand>> = StaticCell::new();
/// ```
pub struct StickySignal<M, T, const WAKERS: usize>
where
    M: RawMutex,
{
    state: Mutex<M, RefCell<State<T, WAKERS>>>,
    // Note: this will wrap so if we have an exceptionally selective signal it may cause bugs
    id: AtomicU16,
    name: Option<&'static str>,
}

impl<M, T, const WAKERS: usize> StickySignal<M, T, WAKERS>
where
    M: RawMutex,
{
    /// Create a new `StickySignal`.
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(RefCell::new(State::new())),
            id: AtomicU16::new(0),
            name: None,
        }
    }

    pub const fn new_with_name(name: &'static str) -> Self {
        Self {
            state: Mutex::new(RefCell::new(State::new())),
            id: AtomicU16::new(0),
            name: Some(name),
        }
    }

    fn prefix(&self) -> &'static str {
        self.name.unwrap_or("signal")
    }

    fn drop_waiter(&self, id: u16) {
        self.state.lock(|cell| {
            let mut cell = cell.borrow_mut();
            defmt::trace!(
                "{}: dropping waiter '{}' ({} total)",
                self.prefix(),
                id,
                cell.waiters.len()
            );

            // swamp remove is faster than retain
            if let Some((idx, _)) = cell.waiters.iter().enumerate().find(|(_, (i, _))| *i != id) {
                cell.waiters.swap_remove(idx);
            }
        })
    }

    /// Mark this StickySignal as signaled.
    pub fn signal(&self, val: T) {
        self.state.lock(|cell| {
            let mut cell = cell.borrow_mut();
            for state in cell.waiters.iter_mut() {
                let old = core::mem::replace(state, (state.0, StateInner::Signaled));
                if let (_, StateInner::Waiting(waker)) = old {
                    waker.wake();
                }
            }
            cell.value = Some(val);
        })
    }

    /// Remove the queued value in this `StickySignal`, if any.
    pub fn reset(&self) {
        self.state.lock(|cell| {
            cell.borrow_mut().value = None;
        });
    }

    /// non-blocking method to try and take a reference to the signal value.
    pub fn try_take(&self) -> Option<T> {
        self.state.lock(|cell| {
            let mut cell = cell.borrow_mut();
            cell.value.take()
        })
    }
}

impl<M, T, const WAKERS: usize> Default for StickySignal<M, T, WAKERS>
where
    M: RawMutex,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<M, T: Send, const WAKERS: usize> StickySignal<M, T, WAKERS>
where
    M: RawMutex,
    T: Clone,
{
    fn poll_wait(&self, name: &'static str, id: u16, cx: &mut Context<'_>) -> Poll<T> {
        self.state.lock(|cell| {
            let mut s = cell.borrow_mut();

            let state = s
                .waiters
                .iter_mut()
                .enumerate()
                .find(|(_, state)| state.0 == id);

            match state {
                Some((_, (_, StateInner::Waiting(_)))) => Poll::Pending,
                Some((idx, (_, StateInner::Signaled))) => {
                    defmt::trace!(
                        "{}: removing idx {} on len {}",
                        self.prefix(),
                        idx,
                        s.waiters.len()
                    );
                    s.waiters.swap_remove(idx);
                    Poll::Ready(s.value.clone().unwrap())
                }
                None => {
                    s.waiters
                        .push((id, StateInner::Waiting(cx.waker().clone())))
                        .unwrap();
                    defmt::trace!(
                        "{}: registering waiter '{}' ({} total)",
                        self.prefix(),
                        name,
                        s.waiters.len()
                    );
                    Poll::Pending
                }
            }
        })
    }

    /// Future that completes when this StickySignal has been signaled.
    pub fn wait(&self, name: &'static str) -> Waiter<'_, M, T, WAKERS> {
        let id = self.id.fetch_add(1, Ordering::Relaxed);
        Waiter {
            id,
            name,
            signal: self,
        }
    }

    /// Future that completes when f returns Some(U). This will also check
    /// the current value.
    pub async fn wait_for<U>(&self, name: &'static str, f: impl Fn(T) -> Option<U>) -> U {
        if let Some(val) = self.peek().and_then(&f) {
            return val;
        }

        loop {
            let val = self.wait(name).await;
            if let Some(val) = f(val) {
                defmt::trace!("{}: got value for '{}'", self.prefix(), name);
                return val;
            }
            defmt::trace!("{}: no value for '{}', waiting", self.prefix(), name);
        }
    }

    /// Check if the StickySignal has been signaled.
    ///
    /// This method returns `true` if the signal has been set, and `false` otherwise.
    // pub fn is_signaled(&self) -> bool {
    //     self.state
    //         .lock(|cell| matches!(*cell.borrow(), State::Signaled(_)))
    // }

    /// Peek at the value in this `StickySignal` without taking it.
    ///
    /// This method returns `Some(&T)` if the signal has been set, and `None` otherwise.
    pub fn peek(&self) -> Option<T> {
        self.state.lock(|cell| cell.borrow().value.clone())
    }
}

pub struct Waiter<'a, M: RawMutex, T: Clone, const WAKERS: usize> {
    id: u16,
    name: &'static str,
    signal: &'a StickySignal<M, T, WAKERS>,
}

// TODO: avoid calling drop_waiter if the future has completed
impl<'a, M: RawMutex, T: Clone, const WAKERS: usize> Drop for Waiter<'a, M, T, WAKERS> {
    fn drop(&mut self) {
        self.signal.drop_waiter(self.id);
    }
}

// NOTE: this future is not 'fused' meaning it cannot be polled after completion
impl<'a, M: RawMutex, T: Clone + Send, const WAKERS: usize> Future for Waiter<'a, M, T, WAKERS> {
    type Output = T;

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.signal.poll_wait(self.name, self.id, cx)
    }
}
