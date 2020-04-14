use std::env;
use std::fmt;

use chrono;
use tracing_subscriber;
use tracing_subscriber::fmt::time::FormatTime;

pub fn init_tracing() {
    let json = if let Some(format) = env::var_os("MIRAKC_LOG_FORMAT") {
        format == "json"
    } else {
        false
    };

    let hrtime = if let Some(timestamp) = env::var_os("MIRAKC_LOG_TIMESTAMP") {
        timestamp == "hrtime"
    } else {
        false
    };

    if json {
        init_json_tracing(hrtime);
    } else {
        init_text_tracing(hrtime);
    }
}

fn init_json_tracing(hrtime: bool) {
    let builder = tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env());
    if hrtime {
        builder.with_timer(HrTime).init();
    } else {
        builder.init();
    }
}

fn init_text_tracing(hrtime: bool) {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env());
    if hrtime {
        builder.with_timer(HrTime).init();
    } else {
        builder.init();
    }
}

struct HrTime;

impl FormatTime for HrTime {
    fn format_time(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        const NANOS_IN_SEC: i64 = 1_000_000_000;
        let ts = chrono::Utc::now().timestamp_nanos();
        let secs = ts / NANOS_IN_SEC;
        let nanos = ts & NANOS_IN_SEC;
        write!(w, "{}.{:>09}", secs, nanos)
    }
}
