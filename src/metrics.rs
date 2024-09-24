use once_cell::sync::{Lazy, OnceCell};
use prometheus::{
    histogram_opts, opts, register_counter_vec_with_registry, register_gauge_vec_with_registry,
    register_histogram_vec_with_registry, CounterVec, GaugeVec, HistogramVec, TextEncoder,
};

static GLOBAL_SETTINGS: OnceCell<GlobalSettings> = OnceCell::new();

// *_MP: Broken out by HTTP method and path.
// These are the crate's original metrics and arguably not as usefel at _SM(C).
// *_SM: Broken out by gRPC service name and method name.
// *_SMC: Broken out by gRPC service name, method name, and result status code.

pub(crate) static COUNTER_MP: Lazy<CounterVec> = Lazy::new(|| {
    let opts = opts!(COUNTER_MP_NAME, COUNTER_DESCRIPTION);
    register_counter_vec_with_registry!(opts, &["method", "path"], get_settings().registry.clone())
        .expect("failed to init counter_mp")
});

pub(crate) static COUNTER_SM: Lazy<CounterVec> = Lazy::new(|| {
    let opts = opts!(COUNTER_SM_NAME, COUNTER_STARTED_DESCRIPTION);
    register_counter_vec_with_registry!(
        opts,
        &["grpc_service", "grpc_method"],
        get_settings().registry.clone()
    )
    .expect("failed to init counter_smc")
});

pub(crate) static COUNTER_SMC: Lazy<CounterVec> = Lazy::new(|| {
    let opts = opts!(COUNTER_SMC_NAME, COUNTER_DESCRIPTION);
    register_counter_vec_with_registry!(
        opts,
        &["grpc_service", "grpc_method", "grpc_code"],
        get_settings().registry.clone()
    )
    .expect("failed to init counter_smc")
});

pub(crate) static HISTOGRAM_MP: Lazy<HistogramVec> = Lazy::new(|| {
    let opts = histogram_opts!(
        HISTOGRAM_MP_NAME,
        HISTOGRAM_DESCRIPTION,
        get_settings().histogram_buckets.clone()
    );
    register_histogram_vec_with_registry!(
        opts,
        &["method", "path"],
        get_settings().registry.clone()
    )
    .expect("failed to init histogram_mp")
});

pub(crate) static HISTOGRAM_SMC: Lazy<HistogramVec> = Lazy::new(|| {
    let opts = histogram_opts!(
        HISTOGRAM_SMC_NAME,
        HISTOGRAM_DESCRIPTION,
        get_settings().histogram_buckets.clone()
    );
    register_histogram_vec_with_registry!(
        opts,
        &["grpc_service", "grpc_method", "grpc_code"],
        get_settings().registry.clone()
    )
    .expect("failed to init histogram_smc")
});

pub(crate) static GAUGE_MP: Lazy<GaugeVec> = Lazy::new(|| {
    let opts = opts!(GAUGE_MP_NAME, GAUGE_DESCRIPTION);
    register_gauge_vec_with_registry!(opts, &["method", "path"], get_settings().registry.clone())
        .expect("failed to init gauge")
});

// Backward compatibility metrics
const COUNTER_MP_NAME: &str = "function_calls_total";
const HISTOGRAM_MP_NAME: &str = "function_calls_duration_seconds";
const GAUGE_MP_NAME: &str = "function_calls_concurrent";

// Metrics that mirror the ones commonly used in Go:
// https://github.com/grpc-ecosystem/go-grpc-middleware/blob/main/providers/prometheus/server_metrics.go
const COUNTER_SM_NAME: &str = "grpc_server_started_total";
const COUNTER_SMC_NAME: &str = "grpc_server_handled_total";
const HISTOGRAM_SMC_NAME: &str = "grpc_server_handling_seconds";

const COUNTER_STARTED_DESCRIPTION: &str = "Total number of RPCs started on the server.";
const COUNTER_DESCRIPTION: &str =
    "Total number of RPCs completed on the server, regardless of success or failure.";
const HISTOGRAM_DESCRIPTION: &str = "Histogram for tracking function call duration";
const GAUGE_DESCRIPTION: &str = "Gauge for tracking concurrent function calls";

const DEFAULT_HISTOGRAM_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

pub(crate) fn get_settings() -> &'static GlobalSettings {
    GLOBAL_SETTINGS.get_or_init(Default::default)
}

/// Initialize the global Prometheus settings.
///
/// You should not call this function if you want to use default settings.
pub fn try_init_settings(settings: GlobalSettings) -> Result<(), Error> {
    GLOBAL_SETTINGS
        .try_insert(settings)
        .map_err(|_| Error::AlreadyInitialized)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Settings have already been initialized")]
    AlreadyInitialized,
    #[error(transparent)]
    PrometheusEncoding(#[from] prometheus::Error),
}

pub struct GlobalSettings {
    pub registry: prometheus::Registry,
    pub histogram_buckets: Vec<f64>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        GlobalSettings {
            histogram_buckets: DEFAULT_HISTOGRAM_BUCKETS.to_vec(),
            registry: prometheus::Registry::new(),
        }
    }
}

impl GlobalSettings {
    fn encode_metrics(&self) -> Result<String, Error> {
        let mut output = String::new();

        TextEncoder::new()
            .encode_utf8(&self.registry.gather(), &mut output)
            .map_err(Error::PrometheusEncoding)?;

        Ok(output)
    }
}

/// Export the collected metrics to the Prometheus format.
pub fn encode_to_string() -> Result<String, Error> {
    get_settings().encode_metrics()
}
