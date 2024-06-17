//!
//! 
//! # Tonic Prometheus Layer
//! A lightweight Prometheus metrics layer for Tonic gRPC Server
//!
//! ## Usage
//!
//! Add `tonic_prometheus_layer` to your `Cargo.toml`.
//! ```not_rust
//! [dependencies]
//! tonic_prometheus_layer = "0.1.3"
//! ```
//!
//! Then add a new layer to your tonic instance like:
//! ```rust,no_run
//! use std::net::SocketAddr;
//! use std::str::FromStr;
//! 
//! use rocket::{get, routes};
//! use rocket::http::Status;
//! use rocket::response::content::RawText;
//! use rocket::config::Shutdown;
//! use rocket::response::status::Custom;
//! use tonic_prometheus_layer::metrics::GlobalSettings;
//! 
//! use crate::api::server;
//! use crate::proto::service_server::ServiceServer;
//! 
//! mod api;
//! mod proto;
//! 
//! #[tokio::main]
//! async fn main() {
//!     let addr: SocketAddr = "127.0.0.1:9090".parse().unwrap();
//! 
//!     let service = server::Server {};
//! 
//!     tonic_prometheus_layer::metrics::try_init_settings(GlobalSettings {
//!         histogram_buckets: vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0],
//!         ..Default::default()
//!     }).unwrap();
//! 
//!     let metrics_layer = tonic_prometheus_layer::MetricsLayer::new();
//! 
//!     tokio::spawn(async {
//!         run_http_server("127.0.0.1:8090").await
//!     });
//! 
//!     tonic::transport::Server::builder()
//!         .layer(metrics_layer)
//!         .add_service(ServiceServer::new(service))
//!         .serve(addr.into())
//!         .await
//!         .unwrap();
//! }
//! 
//! #[get("/metrics")]
//! async fn metrics() -> Custom<RawText<String>> {
//!     let body = tonic_prometheus_layer::metrics::encode_to_string().unwrap();
//! 
//!     Custom(Status::Ok, RawText(body))
//! }
//! 
//! pub async fn run_http_server(addr: &str) {
//!     let addr = SocketAddr::from_str(addr).unwrap();
//! 
//!     let config = rocket::config::Config {
//!         address: addr.ip(),
//!         port: addr.port(),
//!         shutdown: Shutdown {
//!             ctrlc: false,
//!             ..Default::default()
//!         },
//!         ..rocket::config::Config::release_default()
//!     };
//! 
//!     rocket::custom(config)
//!         .mount("/", routes![metrics])
//!         .launch()
//!         .await
//!         .unwrap();
//! }
//! ```
//!
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use pin_project::pin_project;
use tonic::codegen::http::request;
use tonic::transport;
use tower::{Layer, Service};

use crate::metrics::{COUNTER, GAUGE, HISTOGRAM};

pub mod metrics;

#[derive(Clone, Default)]
pub struct MetricsLayer {}

impl MetricsLayer {
    pub fn new() -> Self {
        Default::default()
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService { service: inner }
    }
}

#[derive(Clone)]
pub struct MetricsService<S> {
    service: S,
}

impl<S> Service<request::Request<transport::Body>> for MetricsService<S>
where
    S: Service<request::Request<transport::Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = MetricsFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: request::Request<transport::Body>) -> Self::Future {
        let method = req.method().to_string();
        let path = req.uri().path().to_owned();
        let f = self.service.call(req);

        MetricsFuture::new(method, path, f)
    }
}

#[pin_project]
pub struct MetricsFuture<F> {
    method: String,
    path: String,
    started_at: Option<Instant>,
    #[pin]
    inner: F,
}

impl<F> MetricsFuture<F> {
    pub fn new(method: String, path: String, inner: F) -> Self {
        Self {
            started_at: None,
            inner,
            method,
            path,
        }
    }
}

impl<F, T, E> Future for MetricsFuture<F>
where
    F: Future<Output = Result<T, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let started_at = this.started_at.get_or_insert_with(|| {
            GAUGE.with_label_values(&[this.method, this.path]).inc();

            Instant::now()
        });

        if let Poll::Ready(v) = this.inner.poll(cx) {
            let elapsed = Instant::now().duration_since(*started_at).as_secs_f64();
            COUNTER.with_label_values(&[this.method, this.path]).inc();
            HISTOGRAM
                .with_label_values(&[this.method, this.path])
                .observe(elapsed);
            GAUGE.with_label_values(&[this.method, this.path]).dec();

            Poll::Ready(v)
        } else {
            Poll::Pending
        }
    }
}
