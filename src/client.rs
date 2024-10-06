use pin_project::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::codegen::http::{Request, Response};
use tonic::{Code, GrpcMethod};
use tower::Service;

use crate::metrics::{CLIENT_COUNTER_HANDLED, CLIENT_COUNTER_STARTED, CLIENT_HISTOGRAM};

#[pin_project]
pub struct MetricsChannelFuture<F> {
    service: String,
    method: String,
    started_at: Option<Instant>,
    #[pin]
    inner: F,
}

impl<F> MetricsChannelFuture<F> {
    pub fn new(service: String, method: String, inner: F) -> Self {
        Self {
            inner,
            started_at: None,
            service,
            method,
        }
    }
}

impl<F, O, E> Future for MetricsChannelFuture<F>
where
    F: Future<Output = Result<Response<O>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let started_at = this.started_at.get_or_insert_with(|| {
            CLIENT_COUNTER_STARTED
                .with_label_values(&[this.service, this.method])
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
            CLIENT_COUNTER_HANDLED
                .with_label_values(&[this.service, this.method, &code_str])
                .inc();
            CLIENT_HISTOGRAM
                .with_label_values(&[this.service, this.method, &code_str])
                .observe(elapsed);
            Poll::Ready(v)
        } else {
            Poll::Pending
        }
    }
}

/// Wrapper for instrumenting a tonic client channel with gRPC metrics.
pub struct MetricsChannel<T> {
    inner: T,
}

impl<T> MetricsChannel<T> {
    /// Wrap a channel so that sending RPCs over it increments gRPC client
    /// Prometeus metrics.
    ///
    /// ```
    /// #[tokio::main]
    /// async fn main() {
    ///     let channel = tonic::transport::Channel::from_static("http://localhost")
    ///         .connect()
    ///         .await
    ///         .unwrap();
    ///     let channel = tonic_prometheus_layer::MetricsChannel::new(channel);
    ///     let mut client = tonic_health::pb::health_client::HealthClient::new(channel);
    /// }
    /// ```
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<I, O, T> Service<Request<I>> for MetricsChannel<T>
where
    T: Service<Request<I>, Response = Response<O>>,
    T::Future: Future<Output = Result<T::Response, T::Error>>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = MetricsChannelFuture<T::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<I>) -> Self::Future {
        let (service, method) = req
            .extensions()
            .get::<GrpcMethod>()
            .map_or(("", ""), |gm| (gm.service(), gm.method()));
        MetricsChannelFuture::new(service.into(), method.into(), self.inner.call(req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tonic_health::pb::health_check_response::ServingStatus;
    use tonic_health::pb::{health_client, HealthCheckRequest};

    #[tokio::test]
    async fn simple() {
        let (mut health_reporter, health_service) = tonic_health::server::health_reporter();
        health_reporter
            .set_service_status("yes", tonic_health::ServingStatus::Serving)
            .await;
        let channel = MetricsChannel::new(health_service);
        let mut client = health_client::HealthClient::new(channel);
        let resp = client
            .check(HealthCheckRequest {
                service: String::from("yes"),
                ..Default::default()
            })
            .await
            .expect("Health.Check()")
            .into_inner();
        assert_eq!(resp.status, ServingStatus::Serving as i32);
        let resp = client
            .check(HealthCheckRequest {
                service: String::from("unknown"),
                ..Default::default()
            })
            .await
            .expect_err("Health.Check()");
        assert_eq!(resp.code(), Code::NotFound);

        let got = crate::metrics::encode_to_string().unwrap();
        assert!(got.contains(
            "\ngrpc_client_handled_total{grpc_code=\"NotFound\",grpc_method=\"Check\",grpc_service=\"grpc.health.v1.Health\"} 1\n"));
        assert!(got.contains(
            "\ngrpc_client_handled_total{grpc_code=\"Ok\",grpc_method=\"Check\",grpc_service=\"grpc.health.v1.Health\"} 1\n"));
    }
}
