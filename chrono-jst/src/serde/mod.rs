pub mod duration_milliseconds;
pub mod duration_milliseconds_option;
pub mod ts_milliseconds;
pub mod ts_milliseconds_option;

use chrono::Duration;
use std::fmt;

struct DurationVisitor;

impl<'de> serde::de::Visitor<'de> for DurationVisitor {
    type Value = Duration;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a duration in milliseconds")
    }

    fn visit_i64<E>(self, value: i64) -> Result<Duration, E>
    where
        E: serde::de::Error,
    {
        Ok(Duration::milliseconds(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Duration, E>
    where
        E: serde::de::Error,
    {
        match i64::try_from(value) {
            Ok(v) => Ok(Duration::milliseconds(v)),
            Err(e) => Err(serde::de::Error::custom(e)),
        }
    }
}
