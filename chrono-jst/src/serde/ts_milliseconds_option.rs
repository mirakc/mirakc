// See chrono::serde::ts_milliseconds_option.

use chrono::DateTime;
use chrono::Utc;

use crate::jst::Jst;

pub fn serialize<S>(opt: &Option<DateTime<Jst>>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    chrono::serde::ts_milliseconds_option::serialize(&opt.map(|jst| jst.with_timezone(&Utc)), s)
}

pub fn deserialize<'de, D>(d: D) -> Result<Option<DateTime<Jst>>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    Ok(chrono::serde::ts_milliseconds_option::deserialize(d)?.map(|utc| utc.with_timezone(&Jst)))
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Data {
        #[serde(with = "crate::serde::ts_milliseconds_option")]
        datetime_some: Option<DateTime<Jst>>,
        #[serde(with = "crate::serde::ts_milliseconds_option")]
        datetime_none: Option<DateTime<Jst>>,
    }

    impl Data {
        fn new(timestamp: i64) -> Self {
            Data {
                datetime_some: Some(Utc.timestamp_opt(timestamp, 0).unwrap().with_timezone(&Jst)),
                datetime_none: None,
            }
        }
    }

    const JSON: &'static str = r#"{"datetime_some":1500000000000,"datetime_none":null}"#;
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
// </coverage:exclude>
