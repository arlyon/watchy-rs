# sticky-signal

A `no_std` compatible, single-slot signaling primitive that retains its value after being read. This is useful in asynchronous contexts where multiple tasks might need to observe the latest signaled value, even if they check it at different times.

## Overview

`StickySignal` is similar to `embassy_sync::signal::Signal` but with a key difference: when a waiter receives the signal, the underlying value is *not* cleared. This "stickiness" allows the value to be peeked at or waited for by multiple consumers until it's explicitly reset or overwritten by a new signal.

It's designed for scenarios where a state or command needs to be broadcast and remain active until explicitly changed.

## Features

* **`no_std` Compatibility**: Works in environments without the standard library.
* **Asynchronous**: Designed for use with async executors like Embassy.
* **Value Retention**: Signaled values are retained until explicitly reset or a new value is signaled.
* **Multiple Waiters**: Multiple tasks can wait for or peek at the signaled value.
* **Generic Mutex**: Uses a `RawMutex` trait, allowing flexibility in the choice of mutex implementation (e.g., `CriticalSectionRawMutex` for interrupt-safe contexts, `NoopRawMutex` for single-threaded scenarios).
* **Configurable Waker Capacity**: The maximum number of concurrent waiters can be defined at compile time.
* **Optional Naming**: Signals can be named for easier debugging with `defmt`.

## Core Components

* **`StickySignal<M, T, const WAKERS>`**: The main struct.
    * `M`: The `RawMutex` implementation.
    * `T`: The type of the value to be signaled. Must be `Clone`.
    * `WAKERS`: The maximum number of tasks that can be waiting for the signal simultaneously.
* **`Waiter<'a, M, T, WAKERS>`**: A future that completes when the `StickySignal` is signaled.

## API
 * StickySignal::new(): Creates a new signal.
 * StickySignal::new_with_name(name: &'static str): Creates a new signal with a debug name.
 * StickySignal::signal(&self, val: T): Sets the signal's value and wakes all waiting tasks. T must be Clone.
 * StickySignal::reset(&self): Clears the signaled value.
 * StickySignal::try_take(&self) -> Option<T>: Takes the value out of the signal, leaving it empty.
 * StickySignal::wait(&self, name: &'static str) -> Waiter<'_, M, T, WAKERS>: Returns a future that resolves when the signal is next set. The name is used for debugging.
 * StickySignal::wait_for<U>(&self, name: &'static str, f: impl Fn(T) -> Option<U>) -> U: Waits for the signal and then applies the function f. If f returns Some(value), this future resolves with value. Checks the current value first.
 * StickySignal::peek(&self) -> Option<T>: Returns a clone of the current value if the signal has been set, without waiting or consuming it.
 * Waiter::drop(): Automatically deregisters the waiter from the signal if the Waiter future is dropped before completion.
Internal State
The signal manages an Option<T> for the value and a heapless::Vec of wakers. When signal is called:
 * The new value is stored.
 * All registered wakers are moved to a "Signaled" state and their tasks are woken.
   When a Waiter is polled:
 * If its state is "Signaled", it retrieves a clone of the value and completes. The waiter is removed from the list.
 * If its state is "Waiting", it remains pending.
 * If it's a new waiter, it registers itself with the current task's waker.
The id field in StickySignal and Waiter helps manage individual waiters, ensuring correct removal upon completion or drop. Note that this ID can wrap, which might lead to issues in scenarios with extremely high churn of very selective waiters, though this is generally unlikely.

## License

This crate is licensed under terms compatible with the parent watchy-rs project (e.g., MIT or Apache 2.0). Please refer to the main project's license.
