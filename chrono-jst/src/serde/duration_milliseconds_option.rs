// This module provides serde implementaion for chrono::Duration, which can be
// applied with the `with` field attribute.

use std::fmt;

use chrono::Duration;

use super::DurationVisitor;

pub fn serialize<S>(opt: &Option<Duration>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    match *opt {
        Some(ref duration) => s.serialize_i64(duration.num_milliseconds()),
        None => s.serialize_none(),
    }
}

pub fn deserialize<'de, D>(d: D) -> Result<Option<Duration>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    d.deserialize_option(OptionDurationVisitor)
}

struct OptionDurationVisitor;

impl<'de> serde::de::Visitor<'de> for OptionDurationVisitor {
    type Value = Option<Duration>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a duration in milliseconds or none")
    }

    fn visit_some<D>(self, d: D) -> Result<Self::Value, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        d.deserialize_i64(DurationVisitor).map(Some)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(None)
    }
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use chrono::Duration;
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Data {
        #[serde(with = "crate::serde::duration_milliseconds_option")]
        duration_some: Option<Duration>,
        #[serde(with = "crate::serde::duration_milliseconds_option")]
        duration_none: Option<Duration>,
    }

    impl Data {
        fn new(ms: i64) -> Self {
            Data {
                duration_some: Some(Duration::milliseconds(ms)),
                duration_none: None,
            }
        }
    }

    const JSON: &'static str = r#"{"duration_some":1234,"duration_none":null}"#;
    const DURATION: i64 = 1234;

    #[test]
    fn test_deserialize() {
        assert_eq!(
            Data::new(DURATION),
            serde_json::from_str::<Data>(JSON).unwrap()
        );

        assert!(serde_json::from_str::<Data>(&format!(
            r#"{{"duration_some":{},"duration_none":null}}"#,
            u64::max_value()
        ))
        .is_err());
    }

    #[test]
    fn test_serialize() {
        assert_eq!(JSON, serde_json::to_string(&Data::new(DURATION)).unwrap());
    }
}
// </coverage:exclude>
