#[cfg(feature = "api")]
pub mod api;
#[cfg(feature = "api")]
pub mod metrics_transactions;

/// A macro to conditionally enable metrics-related code.
///
/// This macro wraps the provided code block with a `#[cfg(feature = "metrics")]` attribute.
/// The enclosed code will only be compiled and executed if the `metrics` feature is enabled in the
/// `Cargo.toml` file.
///
/// ## Usage
///
/// If the `metrics` feature is enabled, the code inside the macro will be executed. Otherwise,
/// it will be excluded from the compilation. This is useful for enabling/disabling metrics collection
/// code without cluttering the source with manual feature conditionals.
///
/// ## In `Cargo.toml`
/// The `metrics` feature has to be set in the Cargo.toml of the crate we desire to take metrics from.
///
/// To enable the `metrics` feature, add the following to your `Cargo.toml`:
/// ```toml
/// ethrex-metrics = { path = "./metrics", default-features = false }
/// [features]
/// metrics = ["ethrex-metrics/api"]
/// ```
///
/// In this way, when the `metrics` feature is enabled for that crate, the macro is triggered and the metrics_api is also used.
///
/// Example In Code:
/// ```sh
/// use ethrex_metrics::metrics;
// #[cfg(feature = "metrics")]
// use ethrex_metrics::metrics_transactions::{METRICS_TX};
///
/// metrics!(METRICS_TX.inc());
/// ```
///
/// If you build without the `metrics` feature, the code inside `metrics!` will not be compiled, nor will the Prometheus crate.
#[macro_export]
macro_rules! metrics {
    ($($code:tt)*) => {
        #[cfg(feature = "metrics")]
        {
            $($code)*
        }
    };
}

#[derive(Debug, thiserror::Error)]
pub enum MetricsApiError {
    #[error("{0}")]
    TcpError(#[from] std::io::Error),
}
