use hyper::{client::Client, Body, Request};
use tokio_stream::StreamExt;
use tower::ServiceExt;
use tracing::{info, Level};
use tracing_service::ServiceLayer;
use tracing_subscriber::{
    filter, fmt::format::JsonVisitor, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

// nc -l 8080
const SERVER: &str = "http://127.0.0.1:8080";

fn make_visitor(value: &mut String) -> JsonVisitor<'_> {
    JsonVisitor::new(value)
}

#[tokio::main]
async fn main() {
    // Construct the `Service`
    let client = Client::new().map_request(|json_str: String| {
        Request::builder()
            .method("GET")
            .uri(SERVER)
            .body(Body::from(json_str))
            .unwrap()
    });

    // Create the layer
    let (layer, mut responses) = ServiceLayer::new(client, make_visitor);
    let layer = layer.with_filter(filter::Targets::new().with_target("basic", Level::INFO));

    // Spawn the driver
    let driver = async move {
        while let Some(response) = responses.next().await {
            // Do something with response
            println!("{response:?}");
        }
    };
    let handle = tokio::spawn(driver);

    // Initialize the layer
    tracing_subscriber::registry().with(layer).init();

    info!(answer = 42, question = "life, the universe, and everything");

    // Don't exit
    let _ = handle.await;
}
