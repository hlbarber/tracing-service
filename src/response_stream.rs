use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::{ready, Stream};
use futures_util::{stream::Peekable, StreamExt};
use pin_project_lite::pin_project;
use tower::Service;

pin_project! {
    /// A [`Stream`] of [`Service::Response`]s returned by the [`Service`] as `Request`s are passed
    /// through it.
    #[must_use = "the underlying Service will not process requests unless this is being polled"]
    pub struct ResponseStream<Svc, S> where S: Stream, Svc: Service<S::Item> {
        service: Svc,
        #[pin]
        stream: Peekable<S>,
        #[pin]
        call: Option<Svc::Future>
    }
}

impl<Svc, S> ResponseStream<Svc, S>
where
    S: Stream,
    Svc: Service<S::Item>,
{
    pub fn new(service: Svc, stream: S) -> Self {
        Self {
            service,
            stream: stream.peekable(),
            call: None,
        }
    }
}

impl<Svc, S> Stream for ResponseStream<Svc, S>
where
    S: Stream,
    Svc: Service<S::Item>,
{
    type Item = Result<Svc::Response, Svc::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // If there's an existing future then poll it
        if let Some(some) = this.call.as_mut().as_pin_mut() {
            let poll = match some.poll(cx) {
                Poll::Ready(res) => {
                    this.call.set(None);
                    Poll::Ready(Some(res))
                }
                Poll::Pending => Poll::Pending,
            };
            return poll;
        }

        // Return `Pending` if the stream is not ready
        ready!(this.stream.as_mut().poll_peek(cx));

        // Return `Pending` if the service is not ready
        ready!(this.service.poll_ready(cx))?;

        let item = ready!(this.stream.as_mut().poll_next(cx));
        let item = match item {
            Some(item) => item,
            None => return Poll::Ready(None),
        };

        let fut = this.service.call(item);
        this.call.set(Some(fut));

        let fut = this.call.as_mut().as_pin_mut().unwrap();
        match fut.poll(cx) {
            Poll::Ready(res) => {
                this.call.set(None);
                Poll::Ready(Some(res))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
