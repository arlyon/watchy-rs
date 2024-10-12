use core::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use embassy_time::{Duration, Timer};
use futures::{FutureExt, Stream};
use heapless::Vec;
use pin_utils::{unsafe_pinned, unsafe_unpinned};

pub struct StreamThrottle<const N: usize, S: Stream> {
    items: Vec<S::Item, N>,
    timeout: Duration,
    timer: Timer,
    inner: S,
}

impl<const N: usize, S: Stream> Stream for StreamThrottle<N, S> {
    type Item = Vec<S::Item, N>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // take as many ready items off the stream as we have space for
            match self.as_mut().inner().poll_next(cx) {
                Poll::Ready(Some(v)) => self.as_mut().items().push(v).ok().unwrap(),
                Poll::Ready(None) if self.items.is_empty() => return Poll::Ready(None),
                Poll::Ready(None) => {
                    return Poll::Ready(Some(mem::replace(self.as_mut().items(), Vec::new())));
                }
                Poll::Pending => break,
            }

            if self.items.is_full() {
                break;
            }
        }

        if self.items.is_full() {
            return Poll::Ready(Some(mem::replace(self.as_mut().items(), Vec::new())));
        }

        let poll = {
            let timer = self.as_mut().timer();
            timer.poll_unpin(cx)
        };

        match poll {
            Poll::Ready(()) if self.items.is_empty() => Poll::Pending,
            Poll::Ready(()) => {
                let timeout = self.timeout;
                *self.as_mut().timer() = Timer::after(timeout);
                Poll::Ready(Some(mem::replace(self.as_mut().items(), Vec::new())))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

pub trait StreamExt: Stream + Sized {
    fn throttle<const N: usize>(self, timeout: Duration) -> StreamThrottle<N, Self> {
        StreamThrottle {
            items: Vec::new(),
            timer: Timer::after(timeout),
            timeout,
            inner: self,
        }
    }
}

impl<S: Stream> StreamExt for S {}

impl<const N: usize, S: Stream> StreamThrottle<N, S> {
    unsafe_pinned!(inner: S);
    unsafe_unpinned!(items: Vec<S::Item, N>);
    unsafe_unpinned!(timer: Timer);
}
