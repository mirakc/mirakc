use std::fmt;

use atty;
use chrono;
use tracing_subscriber;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::time::{ChronoLocal, FormatTime};

pub fn init_tracing(format: &str) {
    match format {
        "json" => init_json_tracing(),
        _ => init_text_tracing(),
    }
}

fn init_json_tracing() {
    tracing_subscriber::fmt()
        .json()
        .with_timer(HrTime)
        .with_env_filter(EnvFilter::from_default_env())
        .with_ansi(atty::is(atty::Stream::Stdout))
        .init();
}

fn init_text_tracing() {
    tracing_subscriber::fmt()
        .with_timer(ChronoLocal::rfc3339())
        .with_env_filter(EnvFilter::from_default_env())
        .with_ansi(atty::is(atty::Stream::Stdout))
        .init();
}

struct HrTime;

impl FormatTime for HrTime {
    fn format_time(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        const NANOS_IN_SEC: i64 = 1_000_000_000;
        let ts = chrono::Utc::now().timestamp_nanos();
        let secs = ts / NANOS_IN_SEC;
        let nanos = ts & NANOS_IN_SEC;
        write!(w, "{}.{:09}", secs, nanos)
    }
}
