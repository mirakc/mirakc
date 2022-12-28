use std::fmt;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::LocalResult;
use chrono::NaiveDate;
use chrono::NaiveDateTime;
use chrono::Offset;
use chrono::TimeZone;
use chrono::Utc;

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

    pub fn today() -> NaiveDate {
        Jst::now().date_naive()
    }

    pub fn midnight() -> DateTime<Jst> {
        Jst::today()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Jst)
            .unwrap()
    }
}

impl TimeZone for Jst {
    type Offset = Jst;

    fn from_offset(_offset: &Jst) -> Jst {
        Jst
    }

    fn offset_from_local_date(&self, _local: &NaiveDate) -> LocalResult<Jst> {
        LocalResult::Single(Jst)
    }

    fn offset_from_local_datetime(&self, _local: &NaiveDateTime) -> LocalResult<Jst> {
        LocalResult::Single(Jst)
    }

    fn offset_from_utc_date(&self, _utc: &NaiveDate) -> Jst {
        Jst
    }

    fn offset_from_utc_datetime(&self, _utc: &NaiveDateTime) -> Jst {
        Jst
    }
}

impl Offset for Jst {
    fn fix(&self) -> FixedOffset {
        FixedOffset::east_opt(9 * 60 * 60).unwrap()
    }
}

impl fmt::Display for Jst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.fix(), f) // Simply delegate to FixedOffset
    }
}

impl fmt::Debug for Jst {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.fix(), f) // Simply delegate to FixedOffset
    }
}

// <coverage:exclude>
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
        let jst = Utc.timestamp_opt(UNIX_TIME, 0).unwrap().with_timezone(&Jst);
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
// </coverage:exclude>
