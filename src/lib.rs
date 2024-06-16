use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use pin_project::pin_project;
use tonic::codegen::Body;
use tonic::codegen::http::request;
use tonic::transport;
use tower::{Layer, Service};

use crate::metrics::{COUNTER, GAUGE, HISTOGRAM};

pub mod metrics;

#[derive(Clone)]
pub struct MetricsLayer {}

impl MetricsLayer {
    pub fn new() -> Self {
        Self {}
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            service: inner,
        }
    }
}

#[derive(Clone)]
pub struct MetricsService<S> {
    service: S,
}

impl<S> Service<request::Request<transport::Body>> for MetricsService<S>
    where S: Service<request::Request<transport::Body>> {
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
        Self { started_at: None, inner, method, path }
    }
}

impl<F, T, E> Future for MetricsFuture<F>
    where F: Future<Output=Result<T, E>> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let started_at = this.started_at.get_or_insert_with(|| {
            GAUGE.with_label_values(&[&this.method, &this.path]).inc();

            Instant::now()
        });

        if let Poll::Ready(v) = this.inner.poll(cx) {
            let elapsed = Instant::now().duration_since(*started_at).as_secs_f64();
            COUNTER.with_label_values(&[&this.method, &this.path]).inc();
            HISTOGRAM.with_label_values(&[&this.method, &this.path]).observe(elapsed);
            GAUGE.with_label_values(&[&this.method, &this.path]).dec();

            Poll::Ready(v)
        } else {
            Poll::Pending
        }
    }
}