mod response_stream;

pub use response_stream::*;

use std::fmt;

use tokio::sync::mpsc::{channel, Sender};
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
pub struct ServiceLayer<Request, MakeVisitor> {
    make_visitor: MakeVisitor,
    sender: Sender<Request>,
}

impl<Request, MakeVisitor> ServiceLayer<Request, MakeVisitor> {
    const DEFAULT_BUFFER: usize = 32;

    /// Constructs a `ServiceLayer` with an bounded queue being drained into the [`Service`].
    ///
    /// If the number of items overflows the queue capacity it will fail to process logs.
    pub fn new_with_buffer<Svc>(
        service: Svc,
        make_visitor: MakeVisitor,
        buffer: usize,
    ) -> (Self, ResponseStream<Request, Svc>)
    where
        Svc: Service<Request>,
    {
        let (sender, receiver) = channel(buffer);
        let layer = Self {
            sender,
            make_visitor,
        };
        let handle = ResponseStream::new(service, receiver);

        (layer, handle)
    }

    /// Constructs a `ServiceLayer` with an bounded queue being drained into the [`Service`].
    ///
    /// If the number of items overflows the default queue capacity (32) it will fail to process
    /// logs.
    pub fn new<Svc>(service: Svc, visitor: MakeVisitor) -> (Self, ResponseStream<Request, Svc>)
    where
        Svc: Service<Request>,
    {
        Self::new_with_buffer(service, visitor, Self::DEFAULT_BUFFER)
    }
}

impl<S, Request, MakeVisitor> Layer<S> for ServiceLayer<Request, MakeVisitor>
where
    S: Subscriber,
    for<'a> S: LookupSpan<'a>,

    Request: Default + Send + Sync + 'static,

    for<'a> MakeVisitor: field::MakeVisitor<&'a mut Request>,
    MakeVisitor: 'static,
    for<'a> <MakeVisitor as field::MakeVisitor<&'a mut Request>>::Visitor:
        VisitOutput<Result<(), fmt::Error>>,
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

        if self.sender.try_send(request).is_err() {
            // TODO: This can error in two ways, receiver dropped and receiver full (in the case of
            // a bounded sender).
        };
    }
}
