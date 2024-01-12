// This module provides serde implementaion for chrono::Duration, which can be
// applied with the `with` field attribute.

use chrono::Duration;

use super::DurationVisitor;

pub fn serialize<S>(duration: &Duration, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    s.serialize_i64(duration.num_milliseconds())
}

pub fn deserialize<'de, D>(d: D) -> Result<Duration, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    d.deserialize_i64(DurationVisitor)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use serde::Deserialize;
    use serde::Serialize;
    use serde_json;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Data {
        #[serde(with = "crate::serde::duration_milliseconds")]
        duration: Duration,
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
        assert_eq!(
            Data::new(DURATION),
            serde_json::from_str::<Data>(JSON).unwrap()
        );

        assert!(
            serde_json::from_str::<Data>(&format!(r#"{{"duration":{}}}"#, u64::max_value()))
                .is_err()
        );
    }

    #[test]
    fn test_serialize() {
        assert_eq!(JSON, serde_json::to_string(&Data::new(DURATION)).unwrap());
    }
}
