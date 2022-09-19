#![allow(clippy::blacklisted_name)]

mod test_client;
pub use self::test_client::*;

pub fn assert_send<T: Send>() {}
pub fn assert_sync<T: Sync>() {}
pub fn assert_unpin<T: Unpin>() {}

pub struct NotSendSync(*const ());
