// This module provides serde implementaion for chrono::DateTime<Jst>, which can
// be applied with the `with` field attribute.
//
// As described in https://serde.rs/container-attrs.html, the `from` and `into`
// attributes can be applied only to a struct or enum declaration.
//
// Unfortunately, chrono::serde works only with DateTime<Utc>.  Probably, the
// reason of that is that numeric representaions don't include timezone
// information.
//
// But it's possible to implement the `serialize()` and `deserialize()` by
// converting a DataTime<T> object into a DateTime<Utc> object and then applying
// chrono::serde::*::{serialize, deserialize} on the DateTime<T> object like
// below.

use chrono::DateTime;
use chrono::Utc;

use crate::jst::Jst;

pub fn serialize<S>(jst: &DateTime<Jst>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    chrono::serde::ts_milliseconds::serialize(&jst.with_timezone(&Utc), s)
}

pub fn deserialize<'de, D>(d: D) -> Result<DateTime<Jst>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    Ok(chrono::serde::ts_milliseconds::deserialize(d)?.with_timezone(&Jst))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Data {
        #[serde(with = "crate::serde::ts_milliseconds")]
        datetime: DateTime<Jst>,
    }

    impl Data {
        fn new(timestamp: i64) -> Self {
            Data {
                datetime: Utc.timestamp_opt(timestamp, 0).unwrap().with_timezone(&Jst),
            }
        }
    }

    const JSON: &str = r#"{"datetime":1500000000000}"#;
    const UNIX_TIME: i64 = 1_500_000_000;

    #[test]
    fn test_deserialize() {
        assert_eq!(
            Data::new(UNIX_TIME),
            serde_json::from_str::<Data>(JSON).unwrap()
        );
    }

    #[test]
    fn test_serialize() {
        assert_eq!(JSON, serde_json::to_string(&Data::new(UNIX_TIME)).unwrap());
    }
}
