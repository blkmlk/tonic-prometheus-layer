# Tonic Prometheus Layer
A lightweight Prometheus metrics layer for Tonic gRPC Server inspired by [autometrics](https://github.com/autometrics-dev/autometrics-rs)

## Usage

Add `tonic_prometheus_layer` to your `Cargo.toml`.
```not_rust
[dependencies]
tonic_prometheus_layer = "0.1.5"
```

Then add a new layer to your tonic instance like:
```rust,no_run
use std::net::SocketAddr;
use std::str::FromStr;

use rocket::{get, routes};
use rocket::http::Status;
use rocket::response::content::RawText;
use rocket::config::Shutdown;
use rocket::response::status::Custom;
use tonic_prometheus_layer::metrics::GlobalSettings;

use crate::api::server;
use crate::proto::service_server::ServiceServer;

mod api;
mod proto;

#[tokio::main]
async fn main() {
    let addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();

    let service = server::Server {};

    tonic_prometheus_layer::metrics::try_init_settings(GlobalSettings {
        histogram_buckets: vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0],
        ..Default::default()
    }).unwrap();

    let metrics_layer = tonic_prometheus_layer::MetricsLayer::new();

    tokio::spawn(async {
        run_http_server("127.0.0.1:8090").await
    });

    tonic::transport::Server::builder()
        .layer(metrics_layer)
        .add_service(ServiceServer::new(service))
        .serve(addr.into())
        .await
        .unwrap();
}

#[get("/metrics")]
async fn metrics() -> Custom<RawText<String>> {
    let body = tonic_prometheus_layer::metrics::encode_to_string().unwrap();

    Custom(Status::Ok, RawText(body))
}

pub async fn run_http_server(addr: &str) {
    let addr = SocketAddr::from_str(addr).unwrap();

    let config = rocket::config::Config {
        address: addr.ip(),
        port: addr.port(),
        shutdown: Shutdown {
            ctrlc: false,
            ..Default::default()
        },
        ..rocket::config::Config::release_default()
    };

    rocket::custom(config)
        .mount("/", routes![metrics])
        .launch()
        .await
        .unwrap();
}
```
