use once_cell::sync::{Lazy, OnceCell};
use prometheus::{
    histogram_opts, opts, register_counter_vec_with_registry, register_gauge_vec_with_registry,
    register_histogram_vec_with_registry, CounterVec, GaugeVec, HistogramVec, TextEncoder,
};

static GLOBAL_SETTINGS: OnceCell<GlobalSettings> = OnceCell::new();

pub(crate) static COUNTER: Lazy<CounterVec> = Lazy::new(|| {
    let opts = opts!(COUNTER_NAME, COUNTER_DESCRIPTION);
    register_counter_vec_with_registry!(opts, &["method", "path"], get_settings().registry.clone())
        .expect("failed to init counter")
});

pub(crate) static HISTOGRAM: Lazy<HistogramVec> = Lazy::new(|| {
    let opts = histogram_opts!(
        HISTOGRAM_NAME,
        HISTOGRAM_DESCRIPTION,
        get_settings().histogram_buckets.clone()
    );
    register_histogram_vec_with_registry!(
        opts,
        &["method", "path"],
        get_settings().registry.clone()
    )
    .expect("failed to init histogram")
});

pub(crate) static GAUGE: Lazy<GaugeVec> = Lazy::new(|| {
    let opts = opts!(GAUGE_NAME, GAUGE_DESCRIPTION);
    register_gauge_vec_with_registry!(opts, &["method", "path"], get_settings().registry.clone())
        .expect("failed to init gauge")
});

const COUNTER_NAME: &str = "function_calls_total";
const HISTOGRAM_NAME: &str = "function_calls_duration_seconds";
const GAUGE_NAME: &str = "function_calls_concurrent";

const COUNTER_DESCRIPTION: &str = "Counter for tracking function calls";
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
