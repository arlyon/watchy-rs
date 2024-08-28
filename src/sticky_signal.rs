use core::cell::RefCell;
use core::future::{poll_fn, Future};
use core::task::{Context, Poll, Waker};

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::blocking_mutex::Mutex;

// TODO: allow resolving updates after the first
enum State<T> {
    None,
    Waiting(Waker),
    Signaled(T),
    Occupied(T),
    OccupiedWaiting(T, Waker),
}

/// Single-slot signaling primitive that retains the value after being read.
///
/// This is similar to a [`Signal`](crate::signal::Signal), but it does not clear the inner value
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
/// ```
pub struct StickySignal<M, T>
where
    M: RawMutex,
{
    state: Mutex<M, RefCell<State<T>>>,
}

impl<M, T> StickySignal<M, T>
where
    M: RawMutex,
{
    /// Create a new `StickySignal`.
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(RefCell::new(State::None)),
        }
    }
}

impl<M, T> Default for StickySignal<M, T>
where
    M: RawMutex,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<M, T: Send> StickySignal<M, T>
where
    M: RawMutex,
    T: Copy + Clone,
{
    /// Mark this StickySignal as signaled.
    pub fn signal(&self, val: T) {
        self.state.lock(|cell| {
            let state = cell.replace(State::Signaled(val));
            if let State::Waiting(waker) | State::OccupiedWaiting(_, waker) = state {
                waker.wake();
            }
        })
    }

    /// Remove the queued value in this `StickySignal`, if any.
    pub fn reset(&self) {
        self.state.lock(|cell| cell.replace(State::None));
    }

    fn poll_wait(&self, cx: &mut Context<'_>) -> Poll<T> {
        self.state.lock(|cell| {
            let mut s = cell.borrow_mut();
            match &mut *s {
                s @ State::None => {
                    *s = State::Waiting(cx.waker().clone());
                    Poll::Pending
                }
                State::Waiting(w) | State::OccupiedWaiting(_, w) if w.will_wake(cx.waker()) => {
                    Poll::Pending
                }
                s @ State::Waiting(_) => {
                    let State::Waiting(w) = core::mem::replace(s, State::None) else {
                        panic!("will never happen")
                    };
                    *s = State::Waiting(cx.waker().clone());
                    w.wake();
                    Poll::Pending
                }
                s @ State::OccupiedWaiting(_, _) => {
                    let State::OccupiedWaiting(inner, w) = core::mem::replace(s, State::None)
                    else {
                        panic!("will never happen")
                    };
                    *s = State::OccupiedWaiting(inner, cx.waker().clone());
                    w.wake();
                    Poll::Pending
                }
                s @ State::Signaled(_) => {
                    let State::Signaled(inner) = core::mem::replace(s, State::None) else {
                        panic!()
                    };
                    *s = State::Occupied(inner);
                    Poll::Ready(inner)
                }
                s @ State::Occupied(_) => {
                    let State::Occupied(inner) = core::mem::replace(s, State::None) else {
                        panic!()
                    };
                    *s = State::OccupiedWaiting(inner, cx.waker().clone());
                    Poll::Pending
                }
            }
        })
    }

    /// Future that completes when this StickySignal has been signaled.
    pub fn wait(&self) -> impl Future<Output = T> + '_ {
        poll_fn(move |cx| self.poll_wait(cx))
    }

    /// non-blocking method to try and take a reference to the signal value.
    pub fn try_take(&self) -> Option<T> {
        self.state.lock(|cell| match cell.replace(State::None) {
            State::Signaled(res) | State::Occupied(res) | State::OccupiedWaiting(res, _) => {
                Some(res)
            }
            _ => None,
        })
    }

    /// Check if the StickySignal has been signaled.
    ///
    /// This method returns `true` if the signal has been set, and `false` otherwise.
    pub fn is_signaled(&self) -> bool {
        self.state
            .lock(|cell| matches!(*cell.borrow(), State::Signaled(_)))
    }

    /// Peek at the value in this `StickySignal` without taking it.
    ///
    /// This method returns `Some(&T)` if the signal has been set, and `None` otherwise.
    pub fn peek(&self) -> Option<T> {
        self.state.lock(|cell| match &*cell.borrow() {
            State::Signaled(ref res)
            | State::Occupied(ref res)
            | State::OccupiedWaiting(ref res, _) => Some(res.clone()),
            _ => None,
        })
    }
}
