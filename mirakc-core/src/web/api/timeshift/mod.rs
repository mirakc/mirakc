use std::sync::Arc;

use actlet::*;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::State;
use axum::response::Response;
use axum::Json;
use axum::TypedHeader;

use crate::error::Error;
use crate::timeshift;

use super::AppState;
use crate::web::models::*;
use crate::web::qs::Qs;

pub(super) mod recorders;
