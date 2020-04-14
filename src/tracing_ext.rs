use std::fmt;

use chrono;
use tracing_subscriber::fmt::time::FormatTime;

pub struct HrTime;

impl FormatTime for HrTime {
    fn format_time(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        const NANOS_IN_SEC: i64 = 1_000_000_000;
        let ts = chrono::Utc::now().timestamp_nanos();
        let secs = ts / NANOS_IN_SEC;
        let nanos = ts & NANOS_IN_SEC;
        write!(w, "{}.{:>09}", secs, nanos)
    }
}
