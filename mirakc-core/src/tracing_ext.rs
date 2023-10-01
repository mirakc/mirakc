use std::fmt;

use is_terminal::IsTerminal;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;

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
        .with_ansi(std::io::stdout().is_terminal())
        .init();
}

fn init_text_tracing() {
    tracing_subscriber::fmt()
        .with_timer(Rfc3339Micros)
        .with_env_filter(EnvFilter::from_default_env())
        .with_ansi(std::io::stdout().is_terminal())
        .init();
}

struct HrTime;

impl FormatTime for HrTime {
    fn format_time(&self, w: &mut Writer) -> fmt::Result {
        const NANOS_IN_SEC: i64 = 1_000_000_000;
        let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap();
        let secs = ts / NANOS_IN_SEC;
        let nanos = ts & NANOS_IN_SEC;
        write!(w, "{}.{:09}", secs, nanos)
    }
}

struct Rfc3339Micros;

impl FormatTime for Rfc3339Micros {
    fn format_time(&self, w: &mut Writer) -> fmt::Result {
        let time = chrono::Local::now();
        write!(
            w,
            "{}",
            time.to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
        )
    }
}
