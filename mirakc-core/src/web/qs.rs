use axum::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde_qs;

use crate::error::Error;

// axum uses the serde_urlencoded crate for parsing the query in an URL.
// Unfortunately, the Vec support is out of scope for the serde_urlencoded
// crate and it's suggested to use the serde_qs crate.
//
// * nox/serde_urlencoded/issues/46
//
// axum tried to replace the serde_urlencoded crate with the serde_qs
// crate:
//
// * tokio-rs/axum/issues/434
// * tokio-rs/axum/issues/504
// * tokio-rs/axum/issues/940
//
// but the owner decided not to do that finally.
//
// Actually, the serde_qs crate works well with axum without any difficulty as
// you can see in code below.
pub(super) struct Qs<T>(pub T);

#[async_trait]
impl<S, T> FromRequestParts<S> for Qs<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(serde_qs::from_str::<T>(
            parts.uri.query().unwrap_or(""),
        )?))
    }
}
