mod response_stream;

pub use response_stream::*;

use std::{fmt, marker::PhantomData};

use tokio::sync::mpsc::{
    channel,
    error::{SendError, TrySendError},
    unbounded_channel, Sender, UnboundedSender,
};
use tokio_stream::wrappers::{ReceiverStream, UnboundedReceiverStream};
use tower::Service;
use tracing_core::{Event, Subscriber};
use tracing_subscriber::{
    field::{self, VisitOutput},
    layer::Context as LayerContext,
    registry::LookupSpan,
    Layer,
};

/// A [`Layer`] which uses a [`MakeVisitor`](field::MakeVisitor) to construct a `Request` and then
/// sends it to a [`Service<Request>`].
pub struct ServiceLayer<Request, MakeVisitor, Sink = ()> {
    _request: PhantomData<Request>,
    make_visitor: MakeVisitor,
    sink: Sink,
}

impl<Request, MakeVisitor> ServiceLayer<Request, MakeVisitor, UnboundedSender<Request>> {
    /// Constructs a `ServiceLayer` with an unbounded queue being drained into the [`Service`].
    pub fn new_unbounded<Svc>(
        service: Svc,
        make_visitor: MakeVisitor,
    ) -> (Self, ResponseStream<Svc, UnboundedReceiverStream<Request>>)
    where
        Svc: Service<Request>,
    {
        let (sink, stream) = unbounded_channel();
        let stream = UnboundedReceiverStream::new(stream);
        let layer = Self {
            _request: PhantomData,
            sink,
            make_visitor,
        };
        let handle = ResponseStream::new(service, stream);

        (layer, handle)
    }
}

impl<Request, MakeVisitor> ServiceLayer<Request, MakeVisitor, Sender<Request>> {
    const DEFAULT_BUFFER: usize = 32;

    /// Constructs a `ServiceLayer` with an bounded queue being drained into the [`Service`].
    ///
    /// If the number of items overflows the queue capacity it will fail to process logs.
    pub fn new_with_buffer<Svc>(
        service: Svc,
        make_visitor: MakeVisitor,
        buffer: usize,
    ) -> (Self, ResponseStream<Svc, ReceiverStream<Request>>)
    where
        Svc: Service<Request>,
    {
        let (sink, stream) = channel(buffer);
        let stream = ReceiverStream::new(stream);
        let layer = Self {
            _request: PhantomData,
            sink,
            make_visitor,
        };
        let handle = ResponseStream::new(service, stream);

        (layer, handle)
    }

    /// Constructs a `ServiceLayer` with an bounded queue being drained into the [`Service`].
    ///
    /// If the number of items overflows the default queue capacity (32) it will fail to process
    /// logs.
    pub fn new<Svc>(
        service: Svc,
        visitor: MakeVisitor,
    ) -> (Self, ResponseStream<Svc, ReceiverStream<Request>>)
    where
        Svc: Service<Request>,
    {
        Self::new_with_buffer(service, visitor, Self::DEFAULT_BUFFER)
    }
}

mod sealed {
    use super::*;

    /// Allows for polymorphism over [`UnboundedSender`] and [`Sender`].
    pub trait SyncSender<T> {
        type Error;

        fn sink_send(&self, value: T) -> Result<(), Self::Error>;
    }

    impl<T> SyncSender<T> for UnboundedSender<T> {
        type Error = SendError<T>;

        fn sink_send(&self, value: T) -> Result<(), SendError<T>> {
            self.send(value)
        }
    }

    impl<T> SyncSender<T> for Sender<T> {
        type Error = TrySendError<T>;

        fn sink_send(&self, value: T) -> Result<(), Self::Error> {
            self.try_send(value)
        }
    }
}

impl<S, Request, MakeVisitor, Sink> Layer<S> for ServiceLayer<Request, MakeVisitor, Sink>
where
    S: Subscriber,
    for<'a> S: LookupSpan<'a>,

    Request: Default + Send + Sync + 'static,

    for<'a> MakeVisitor: field::MakeVisitor<&'a mut Request>,
    MakeVisitor: 'static,
    for<'a> <MakeVisitor as field::MakeVisitor<&'a mut Request>>::Visitor:
        VisitOutput<Result<(), fmt::Error>>,

    Sink: sealed::SyncSender<Request>,
    Sink: 'static,
{
    // TODO: Add spans

    fn on_event(&self, event: &Event<'_>, _ctx: LayerContext<'_, S>) {
        // Construct the request using the visitor implementation
        let mut request = Request::default();
        let mut visitor = self.make_visitor.make_visitor(&mut request);
        event.record(&mut visitor);

        // There needs to be some consideration on what to do with these errors. Logging them
        // naively might make the situation worse.
        //
        // Allowing the user to provide a backup subscriber to log this might be an avenue.
        if visitor.finish().is_err() {
            // TODO
        };

        if self.sink.sink_send(request).is_err() {
            // TODO: This can error in two ways, receiver dropped and receiver full (in the case of
            // a bounded sender).
        };
    }
}
