//!
//! 
//! # Tonic Prometheus Layer
//! A lightweight Prometheus metrics layer for Tonic gRPC Server
//!
//! It provides the following metrics:
//! * `grpc_server_handled_total`: a **Counter** for tracking the total number of completed gRPC calls.
//! * `grpc_server_started_total`: a **Counter** for tracking the total number of gRPC calls started.
//! The difference between this and `grpc_server_handled_total` equals the number of ongoing requests.
//! * `grpc_server_handling_seconds`: a **Histogram** for tracking gRPC call duration.
//!
//! ## Usage
//!
//! Add `tonic_prometheus_layer` to your `Cargo.toml`.
//! ```not_rust
//! [dependencies]
//! tonic_prometheus_layer = "0.1.6"
//! ```
//!
//! Then add a new layer to your tonic instance:
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
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use pin_project::pin_project;
use tonic::codegen::http::{request, response};
use tonic::Code;
use tower::{Layer, Service};

use crate::metrics::{COUNTER_MP, GAUGE_MP, HISTOGRAM_MP};
use crate::metrics::{COUNTER_SM, COUNTER_SMC, HISTOGRAM_SMC};

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

impl<S, B, C> Service<request::Request<B>> for MetricsService<S>
where
    S: Service<request::Request<B>, Response = response::Response<C>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = MetricsFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, req: request::Request<B>) -> Self::Future {
        let method = req.method().to_string();
        let path = req.uri().path().to_owned();
        let service_method_separator: Option<NonZeroUsize> = match path.chars().next() {
            Some(first_char) if first_char == '/' => path[1..]
                .find('/')
                .map(|p| NonZeroUsize::new(p + 1).unwrap()),
            _ => None,
        };
        let f = self.service.call(req);

        MetricsFuture::new(method, path, service_method_separator, f)
    }
}

#[pin_project]
pub struct MetricsFuture<F> {
    method: String,
    path: String,
    service_method_separator: Option<NonZeroUsize>,
    started_at: Option<Instant>,
    #[pin]
    inner: F,
}

impl<F> MetricsFuture<F> {
    pub fn new(
        method: String,
        path: String,
        service_method_separator: Option<NonZeroUsize>,
        inner: F,
    ) -> Self {
        Self {
            started_at: None,
            inner,
            method,
            path,
            service_method_separator,
        }
    }
}

impl<F, B, E> Future for MetricsFuture<F>
where
    F: Future<Output = Result<response::Response<B>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let (rpc_service, rpc_method) = match this.service_method_separator {
            Some(sep) => (
                &this.path[1..(*sep).into()],
                &this.path[usize::from(*sep) + 1..],
            ),
            // If unparseable, say service is empty and method is the entire path
            None => ("", this.path as &str),
        };

        let started_at = this.started_at.get_or_insert_with(|| {
            GAUGE_MP.with_label_values(&[this.method, this.path]).inc();
            COUNTER_SM
                .with_label_values(&[rpc_service, rpc_method])
                .inc();

            Instant::now()
        });

        if let Poll::Ready(v) = this.inner.poll(cx) {
            let code = v.as_ref().map_or(Code::Unknown, |resp| {
                resp.headers()
                    .get("grpc-status")
                    .map(|s| Code::from_bytes(s.as_bytes()))
                    .unwrap_or(Code::Ok)
            });
            let code_str = format!("{:?}", code);
            let elapsed = Instant::now().duration_since(*started_at).as_secs_f64();
            COUNTER_MP
                .with_label_values(&[this.method, this.path])
                .inc();
            COUNTER_SMC
                .with_label_values(&[rpc_service, rpc_method, &code_str])
                .inc();
            HISTOGRAM_MP
                .with_label_values(&[this.method, this.path])
                .observe(elapsed);
            HISTOGRAM_SMC
                .with_label_values(&[rpc_service, rpc_method, &code_str])
                .observe(elapsed);
            GAUGE_MP.with_label_values(&[this.method, this.path]).dec();

            Poll::Ready(v)
        } else {
            Poll::Pending
        }
    }
}
