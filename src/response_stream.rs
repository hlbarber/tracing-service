use std::{
    future::Future,
    hint::unreachable_unchecked,
    pin::Pin,
    task::{Context, Poll},
};

use futures_core::{ready, Stream};
use pin_project_lite::pin_project;
use tokio::sync::mpsc::Receiver;
use tower::Service;

pin_project! {
    #[project = InnerProj]
    #[project_replace = InnerProjReplace]
    enum Inner<Request, Fut> {
        WaitingStream,
        WaitingService { request: Request },
        Existing { #[pin] future: Fut },
        Closed,
    }
}

pin_project! {
    /// A [`Stream`] of [`Service::Response`]s returned by the [`Service`] as `Request`s are passed
    /// through it.
    #[must_use = "the underlying Service will not process requests unless this is being polled"]
    pub struct ResponseStream<Request, Svc> where Svc: Service<Request> {
        service: Svc,
        #[pin]
        receiver: Receiver<Request>,
        #[pin]
        inner: Inner<Request, Svc::Future>
    }
}

impl<Request, Svc> Stream for ResponseStream<Request, Svc>
where
    Svc: Service<Request>,
{
    type Item = Result<Svc::Response, Svc::Error>;

    #[inline]
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();

        let inner = this.inner.as_mut().project();
        match inner {
            // Waiting for stream to yield a request
            InnerProj::WaitingStream => {
                let item = ready!(this.receiver.poll_recv(cx));

                match item {
                    Some(request) => {
                        this.inner.set(Inner::WaitingService { request });
                        self.poll_next(cx)
                    }
                    None => {
                        this.inner.set(Inner::Closed);
                        Poll::Ready(None)
                    }
                }
            }
            // Waiting for service to be ready, then call it
            InnerProj::WaitingService { .. } => {
                let result = ready!(this.service.poll_ready(cx));

                // We can reuse the Inner::Closed state here as an intermediate state
                let inner = this.inner.as_mut().project_replace(Inner::Closed);
                if let InnerProjReplace::WaitingService { request } = inner {
                    if let Err(err) = result {
                        Poll::Ready(Some(Err(err)))
                    } else {
                        let future = this.service.call(request);
                        this.inner.set(Inner::Existing { future });
                        self.poll_next(cx)
                    }
                } else {
                    // The InnerProj and InnerProjReplace match paths should be identical
                    unsafe { unreachable_unchecked() }
                }
            }
            // Waiting for existing Svc::Future to resolve
            InnerProj::Existing { future } => {
                let output = ready!(future.poll(cx));
                this.inner.set(Inner::WaitingStream);
                Poll::Ready(Some(output))
            }
            // Terminal closed state
            InnerProj::Closed => Poll::Ready(None),
        }
    }
}

impl<Request, Svc> ResponseStream<Request, Svc>
where
    Svc: Service<Request>,
{
    pub(crate) fn new(service: Svc, receiver: Receiver<Request>) -> Self {
        Self {
            service,
            receiver,
            inner: Inner::WaitingStream,
        }
    }
}
