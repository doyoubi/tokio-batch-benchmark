use futures::stream::Stream;
use futures::task::{Context, Poll};
use futures::sink::Sink;
use futures::ready;
use core::fmt;
use core::pin::Pin;

use super::bilock::BiLock;

/// A `Stream` part of the split pair
#[derive(Debug)]
#[must_use = "streams do nothing unless polled"]
pub struct SplitStream<S>(BiLock<S>);

impl<S> Unpin for SplitStream<S> {}

impl<S: Unpin> SplitStream<S> {
    /// Attempts to put the two "halves" of a split `Stream + Sink` back
    /// together. Succeeds only if the `SplitStream<S>` and `SplitSink<S>` are
    /// a matching pair originating from the same call to `Stream::split`.
    #[allow(dead_code)]
    pub fn reunite<Item>(self, other: SplitSink<S, Item>) -> Result<S, ReuniteError<S, Item>>
        where S: Sink<Item>,
    {
        other.reunite(self)
    }
}

impl<S: Stream> Stream for SplitStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        ready!(self.0.poll_lock(cx)).as_pin_mut().poll_next(cx)
    }
}

#[allow(bad_style)]
fn SplitSink<S: Sink<Item>, Item>(lock: BiLock<S>) -> SplitSink<S, Item> {
    SplitSink {
        lock,
        slot: None,
    }
}

/// A `Sink` part of the split pair
#[derive(Debug)]
#[must_use = "sinks do nothing unless polled"]
pub struct SplitSink<S, Item> {
    lock: BiLock<S>,
    slot: Option<Item>,
}

impl<S, Item> Unpin for SplitSink<S, Item> {}

impl<S: Sink<Item> + Unpin, Item> SplitSink<S, Item> {
    /// Attempts to put the two "halves" of a split `Stream + Sink` back
    /// together. Succeeds only if the `SplitStream<S>` and `SplitSink<S>` are
    /// a matching pair originating from the same call to `Stream::split`.
    pub fn reunite(self, other: SplitStream<S>) -> Result<S, ReuniteError<S, Item>> {
        self.lock.reunite(other.0).map_err(|err| {
            ReuniteError(SplitSink(err.0), SplitStream(err.1))
        })
    }
}

impl<S: Sink<Item>, Item> SplitSink<S, Item> {
    fn poll_flush_slot(mut inner: Pin<&mut S>, slot: &mut Option<Item>, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        if slot.is_some() {
            ready!(inner.as_mut().poll_ready(cx))?;
            Poll::Ready(inner.start_send(slot.take().unwrap()))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_lock_and_flush_slot(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        let this = &mut *self;
        let mut inner = ready!(this.lock.poll_lock(cx));
        Self::poll_flush_slot(inner.as_pin_mut(), &mut this.slot, cx)
    }
}

impl<S: Sink<Item>, Item> Sink<Item> for SplitSink<S, Item> {
    type Error = S::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        loop {
            if self.slot.is_none() {
                return Poll::Ready(Ok(()));
            }
            ready!(self.as_mut().poll_lock_and_flush_slot(cx))?;
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Item) -> Result<(), S::Error> {
        self.slot = Some(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        let this = &mut *self;
        let mut inner = ready!(this.lock.poll_lock(cx));
        ready!(Self::poll_flush_slot(inner.as_pin_mut(), &mut this.slot, cx))?;
        inner.as_pin_mut().poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), S::Error>> {
        let this = &mut *self;
        let mut inner = ready!(this.lock.poll_lock(cx));
        ready!(Self::poll_flush_slot(inner.as_pin_mut(), &mut this.slot, cx))?;
        inner.as_pin_mut().poll_close(cx)
    }
}

pub(super) fn split<S: Stream + Sink<Item>, Item>(s: S) -> (SplitSink<S, Item>, SplitStream<S>) {
    let (a, b) = BiLock::new(s);
    let read = SplitStream(a);
    let write = SplitSink(b);
    (write, read)
}

/// Error indicating a `SplitSink<S>` and `SplitStream<S>` were not two halves
/// of a `Stream + Split`, and thus could not be `reunite`d.
pub struct ReuniteError<T, Item>(pub SplitSink<T, Item>, pub SplitStream<T>);

impl<T, Item> fmt::Debug for ReuniteError<T, Item> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ReuniteError")
            .field(&"...")
            .finish()
    }
}

impl<T, Item> fmt::Display for ReuniteError<T, Item> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tried to reunite a SplitStream and SplitSink that don't form a pair")
    }
}

#[cfg(feature = "std")]
impl<T: core::any::Any, Item> std::error::Error for ReuniteError<T, Item> {}
