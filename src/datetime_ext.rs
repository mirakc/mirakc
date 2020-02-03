use std::fmt;

use chrono::{
    DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset,
    TimeZone, Utc,
};

// The following implementation is based on chrono::offset::Utc.
//
// See https://github.com/chronotope/chrono/blob/master/src/offset/utc.rs for
// details.

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Jst;

impl Jst {
    pub fn now() -> DateTime<Jst> {
        Utc::now().with_timezone(&Jst)
    }
}

impl TimeZone for Jst {
    type Offset = Jst;

    fn from_offset(_offset: &Jst) -> Jst { Jst }

    fn offset_from_local_date(&self, _local: &NaiveDate) -> LocalResult<Jst> {
        LocalResult::Single(Jst)
    }

    fn offset_from_local_datetime(
        &self, _local: &NaiveDateTime) -> LocalResult<Jst> {
        LocalResult::Single(Jst)
    }

    fn offset_from_utc_date(&self, _utc: &NaiveDate) -> Jst { Jst }

    fn offset_from_utc_datetime(&self, _utc: &NaiveDateTime) -> Jst { Jst }
}

impl Offset for Jst {
    fn fix(&self) -> FixedOffset { FixedOffset::east(9 * 60 * 60) }
}

impl fmt::Display for Jst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.fix(), f)  // Simply delegate to FixedOffset
    }
}

impl fmt::Debug for Jst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.fix(), f)  // Simply delegate to FixedOffset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC2822_STR: &'static str = "Fri, 14 Jul 2017 11:40:00 +0900";
    const UNIX_TIME: i64 = 1_500_000_000;

    #[test]
    fn test_now() {
        let jst = Jst::now();
        assert_eq!(jst.timezone(), Jst);
    }

    #[test]
    fn test_to_rfc2822() {
        let jst = Utc.timestamp(UNIX_TIME, 0).with_timezone(&Jst);
        assert_eq!(jst.to_rfc2822(), RFC2822_STR);
    }

    #[test]
    fn test_from_rfc2822() {
        let jst = DateTime::parse_from_rfc2822(RFC2822_STR)
                           .unwrap()
                           .with_timezone(&Jst);
        assert_eq!(jst.timestamp(), UNIX_TIME);
    }
}

// The `serde_jst` module provides serde implementaion for
// chrono::DateTime<Jst>, which can be applied with the `with` field attribute.
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
pub mod serde_jst {
    use chrono::{DateTime, Utc};
    use chrono::serde::ts_milliseconds;
    use serde::{ser, de};

    use super::Jst;

    pub fn serialize<S>(
        jst: &DateTime<Jst>, s: S) -> Result<S::Ok, S::Error>
    where S: ser::Serializer
    {
        ts_milliseconds::serialize(&jst.with_timezone(&Utc), s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<DateTime<Jst>, D::Error>
    where D: de::Deserializer<'de>
    {
        Ok(ts_milliseconds::deserialize(d)?.with_timezone(&Jst))
    }

    #[cfg(test)]
    mod tests {
        use chrono::{DateTime, TimeZone, Utc};
        use serde::{Serialize, Deserialize};
        use serde_json;

        use crate::datetime_ext::{serde_jst, Jst};

        #[derive(Debug, Deserialize, Serialize, PartialEq)]
        struct Data {
            #[serde(with = "serde_jst")]
            datetime: DateTime<Jst>
        }

        impl Data {
            fn new(timestamp: i64) -> Self {
                Data {
                    datetime: Utc.timestamp(timestamp, 0).with_timezone(&Jst),
                }
            }
        }

        const JSON: &'static str = r#"{"datetime":1500000000000}"#;
        const UNIX_TIME: i64 = 1_500_000_000;

        #[test]
        fn test_deserialize() {
            assert_eq!(Data::new(UNIX_TIME),
                       serde_json::from_str::<Data>(JSON).unwrap());
        }

        #[test]
        fn test_serialize() {
            assert_eq!(JSON,
                       serde_json::to_string(&Data::new(UNIX_TIME)).unwrap());
        }
    }
}

// The `serde_duration_in_millis` module provides serde implementaion for
// chrono::Duration, which can be applied with the `with` field attribute.
pub mod serde_duration_in_millis {
    use std::fmt;
    use std::convert::TryFrom;

    use chrono::Duration;
    use serde::{ser, de};

    pub fn serialize<S>(
        duration: &Duration, s: S) -> Result<S::Ok, S::Error>
    where S: ser::Serializer
    {
        s.serialize_i64(duration.num_milliseconds())
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Duration, D::Error>
    where D: de::Deserializer<'de>
    {
        d.deserialize_i64(DurationVisitor)
    }

    struct DurationVisitor;

    impl<'de> de::Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a duration in milliseconds")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Duration, E>
        where E: de::Error
        {
            Ok(Duration::milliseconds(value))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Duration, E>
        where E: de::Error
        {
            match i64::try_from(value) {
                Ok(v) => Ok(Duration::milliseconds(v)),
                Err(e) => Err(de::Error::custom(e)),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use chrono::Duration;
        use serde::{Serialize, Deserialize};
        use serde_json;

        use crate::datetime_ext::serde_duration_in_millis;

        #[derive(Debug, Deserialize, Serialize, PartialEq)]
        struct Data {
            #[serde(with = "serde_duration_in_millis")]
            duration: Duration
        }

        impl Data {
            fn new(ms: i64) -> Self {
                Data {
                    duration: Duration::milliseconds(ms),
                }
            }
        }

        const JSON: &'static str = r#"{"duration":1234}"#;
        const DURATION: i64 = 1234;

        #[test]
        fn test_deserialize() {
            assert_eq!(Data::new(DURATION),
                       serde_json::from_str::<Data>(JSON).unwrap());

            assert!(serde_json::from_str::<Data>(
                &format!(r#"{{"duration":{}}}"#, u64::max_value())).is_err());
        }

        #[test]
        fn test_serialize() {
            assert_eq!(JSON,
                       serde_json::to_string(&Data::new(DURATION)).unwrap());
        }
    }
}
