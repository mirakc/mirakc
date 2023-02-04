use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::error::Error;

#[derive(Serialize)]
struct ErrorBody {
    code: u16,
    reason: Option<&'static str>,
    errors: Vec<u8>,
}

macro_rules! error_response {
    ($status_code:expr) => {
        (
            $status_code,
            Json(ErrorBody {
                code: $status_code.as_u16(),
                reason: None,
                errors: vec![],
            }),
        )
            .into_response()
    };
    ($status_code:expr, $reason:literal) => {
        (
            $status_code,
            Json(ErrorBody {
                code: $status_code.as_u16(),
                reason: Some($reason),
                errors: vec![],
            }),
        )
            .into_response()
    };
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        match self {
            Error::StreamingTimedOut => error_response!(StatusCode::REQUEST_TIMEOUT),
            Error::TunerUnavailable => error_response!(StatusCode::NOT_FOUND),
            Error::TunerNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ChannelNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ServiceNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ProgramNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::RecordNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::ScheduleNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::RecorderNotFound => error_response!(StatusCode::NOT_FOUND),
            Error::OutOfRange => error_response!(StatusCode::RANGE_NOT_SATISFIABLE),
            Error::NoContent => error_response!(StatusCode::NO_CONTENT),
            Error::NoLogoData => {
                error_response!(StatusCode::SERVICE_UNAVAILABLE, "Logo Data Unavailable")
            }
            Error::AccessDenied => error_response!(StatusCode::FORBIDDEN),
            Error::AlreadyExists => error_response!(StatusCode::BAD_REQUEST),
            Error::ProgramEnded => error_response!(StatusCode::BAD_REQUEST),
            Error::QuerystringError(_) => error_response!(StatusCode::BAD_REQUEST),
            Error::InvalidPath => error_response!(StatusCode::BAD_REQUEST),
            _ => error_response!(StatusCode::INTERNAL_SERVER_ERROR),
        }
    }
}
