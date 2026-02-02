use super::*;

use std::collections::HashMap;
use std::future::Future;

use assert_matches::assert_matches;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header::ACCEPT_RANGES;
use axum::http::header::CONTENT_LENGTH;
use axum::http::header::CONTENT_RANGE;
use axum::http::header::CONTENT_TYPE;
use axum::http::header::HOST;
use axum::http::header::LOCATION;
use axum::response::Response;
use http_body_util::BodyExt; // for `collect`
use mime::APPLICATION_JSON;
use serde_json::json;
use test_log::test;
use tower::ServiceExt;

use crate::epg::stub::EpgStub;
use crate::models::TunerUserPriority;
use crate::onair::stub::OnairProgramManagerStub;
use crate::recording::RecordingOptions;
use crate::recording::stub::RecordingManagerStub;
use crate::timeshift::stub::TimeshiftManagerStub;
use crate::tuner::stub::TunerManagerStub;
use api::models::*;
use peer_info::PeerInfo;
use qs::*;

// See the following example for how to write tests:
// https://github.com/tokio-rs/axum/blob/main/examples/testing/src/main.rs

macro_rules! to_json {
    ($data:expr) => {
        serde_json::to_string(&$data).unwrap()
    };
}

#[test(tokio::test)]
async fn test_get_unknown() {
    let res = get("/api/unknown").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_version() {
    let res = get("/api/version").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), Some(v) => {
        assert_eq!(v, "close");
    });
    assert_matches!(res.headers().get(CACHE_CONTROL), Some(v) => {
        assert_eq!(v, "no-store");
    });
}

#[test(tokio::test)]
async fn test_get_status() {
    let res = get("/api/status").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_channels() {
    let res = get("/api/channels").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_services() {
    let res = get("/api/services").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_service() {
    let res = get("/api/services/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/services/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_service_logo() {
    let res = get("/api/services/1/logo").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/services/0/logo").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/api/services/2/logo").await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test(tokio::test)]
async fn test_head_service_logo() {
    let res = head("/api/services/1/logo").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = head("/api/services/0/logo").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = head("/api/services/2/logo").await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test(tokio::test)]
async fn test_get_service_programs() {
    let res = get("/api/services/1/programs").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = head("/api/services/0/programs").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_programs() {
    let res = get("/api/programs").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_program() {
    let res = get("/api/programs/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/programs/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_tuners() {
    let res = get("/api/tuners").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_tuner() {
    let res = get("/api/tuners/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/tuners/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_channel_stream() {
    let res = get("/api/channels/GR/ch/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    let res = get("/api/channels/GR/0/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/api/channels/WOWOW/ch/stream").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = get("/api/channels/GR/xx/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let decode_values = [0, 1];

    let valid_pairs = ["GR", "BS", "CS", "SKY"].iter().zip(&decode_values);
    for (channel_type, decode) in valid_pairs {
        let res =
            get(format!("/api/channels/{channel_type}/ch/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    for decode in &decode_values {
        let res = get(format!("/api/channels/WOWOW/ch/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    let res = get("/api/channels/GR/ch/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });
}

#[test(tokio::test)]
async fn test_get_channel_service_stream() {
    let res = get("/api/channels/GR/ch/services/1/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    let res = get("/api/channels/GR/0/services/1/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/api/channels/WOWOW/ch/services/1/stream").await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let decode_values = [0, 1];

    let valid_pairs = ["GR", "BS", "CS", "SKY"].iter().zip(&decode_values);
    for (channel_type, decode) in valid_pairs {
        let res = get(
            format!("/api/channels/{channel_type}/ch/services/1/stream?decode={decode}").as_str(),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    for decode in &decode_values {
        let res =
            get(format!("/api/channels/WOWOW/ch/services/1/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    for decode in &decode_values {
        let res =
            get(format!("/api/channels/WOWOW/ch/services/2/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    let res = get("/api/channels/GR/ch/services/1/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });
}

#[test(tokio::test)]
async fn test_get_service_stream() {
    let res = get("/api/services/1/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    let res = get("/api/services/0/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let decode_values = [0, 1];

    for decode in &decode_values {
        let res = get(format!("/api/services/1/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    for decode in &decode_values {
        let res = get(format!("/api/services/0/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    for decode in &decode_values {
        let res = get(format!("/api/services/2/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    let res = get("/api/services/1/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });
}

#[test(tokio::test)]
async fn test_head_service_stream() {
    let res = head("/api/services/1/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });

    let res = head("/api/services/0/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // See comments in head_service_stream().
    let res = head("/api/services/2/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });

    let res = head("/api/services/1/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });
}

#[test(tokio::test)]
async fn test_get_program_stream() {
    let res = get("/api/programs/100001/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    let res = get("/api/programs/0/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let decode_values = [0, 1];

    for decode in &decode_values {
        let res = get(format!("/api/programs/100001/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    for decode in &decode_values {
        let res = get(format!("/api/programs/0/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    for decode in &decode_values {
        let res = get(format!("/api/programs/200001/stream?decode={decode}").as_str()).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    let res = get("/api/programs/100001/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });
}

#[test(tokio::test)]
async fn test_get_recording_schedules() {
    let res = get("/api/recording/schedules").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_create_recording_schedule() {
    // Error::ProgramNotFound
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 0).into(),
        options: recording_options!("0.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // Error::AlreadyExists
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 1).into(),
        options: recording_options!("1.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Error::ProgramAlreadyStarted
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 2).into(),
        options: recording_options!("2.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Ok
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 4).into(),
        options: recording_options!("4.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::CREATED);

    // Error::InvalidPath
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 4).into(),
        options: recording_options!("/4.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Error::InvalidPath
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 4).into(),
        options: recording_options!("../4.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Use auto-generated filename for the content file.
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 4).into(),
        options: recording_options!(1),
        tags: Default::default(),
    };
    let res = post("/api/recording/schedules", input).await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[test(tokio::test)]
async fn test_get_recording_schedule() {
    let res = get("/api/recording/schedules/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/recording/schedules/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_delete_recording_schedule() {
    let res = delete("/api/recording/schedules/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = delete("/api/recording/schedules/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_delete_recording_schedules() {
    let res = delete("/api/recording/schedules").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = delete("/api/recording/schedules?target=tag").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_recording_recorders() {
    let res = get("/api/recording/recorders").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_create_recording_recorder() {
    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 1).into(),
        options: recording_options!("program.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/recorders", input).await;
    assert_eq!(res.status(), StatusCode::CREATED);

    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 0).into(),
        options: recording_options!("program.m2ts", 1),
        tags: Default::default(),
    };
    let res = post("/api/recording/recorders", input).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let input = WebRecordingScheduleInput {
        program_id: (0, 1, 1).into(),
        options: recording_options!(1),
        tags: Default::default(),
    };
    let res = post("/api/recording/recorders", input).await;
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[test(tokio::test)]
async fn test_get_recording_recorder() {
    let res = get("/api/recording/recorders/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/recording/recorders/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_delete_recording_recorder() {
    let res = delete("/api/recording/recorders/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = delete("/api/recording/recorders/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_recording_records() {
    let res = get("/api/recording/records").await;
    assert_eq!(res.status(), StatusCode::OK);
    let json = into_text(res).await;
    let records: Vec<WebRecord> = serde_json::from_str(&json).unwrap();
    assert_eq!(records.len(), 0);
}

#[test(tokio::test)]
async fn test_get_recording_record() {
    let res = get("/api/recording/records/not-found").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/api/recording/records/finished").await;
    assert_eq!(res.status(), StatusCode::OK);
    let json = into_text(res).await;
    let record: WebRecord = serde_json::from_str(&json).unwrap();
    assert!(record.content.length.is_some());

    let res = get("/api/recording/records/recording").await;
    // Any record now recording can be accessible.
    assert_eq!(res.status(), StatusCode::OK);
    let json = into_text(res).await;
    let record: WebRecord = serde_json::from_str(&json).unwrap();
    assert!(record.content.length.is_some());

    let res = get("/api/recording/records/no-content").await;
    // Any record can be accessible regardless of existence of the recorded data.
    assert_eq!(res.status(), StatusCode::OK);
    // But `WebRecord::size` is `None`.
    let json = into_text(res).await;
    let record: WebRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(record.content.length, None);
}

#[test(tokio::test)]
async fn test_delete_recording_record() {
    let res = delete("/api/recording/records/not-found").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = delete("/api/recording/records/finished").await;
    assert_eq!(res.status(), StatusCode::OK);
    let json = into_text(res).await;
    let result: WebRecordRemovalResult = serde_json::from_str(&json).unwrap();
    assert!(result.record_removed);
    assert!(!result.content_removed);

    let res = delete("/api/recording/records/finished?purge=true").await;
    assert_eq!(res.status(), StatusCode::OK);
    let json = into_text(res).await;
    let result: WebRecordRemovalResult = serde_json::from_str(&json).unwrap();
    assert!(result.record_removed);
    assert!(result.content_removed);

    let res = delete("/api/recording/records/recording").await;
    // Any record now recording cannot be deleted.
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    let res = delete("/api/recording/records/no-content?purge=true").await;
    // Any existing record can be deleted successfully regardless of existence of the recorded
    // data.
    assert_eq!(res.status(), StatusCode::OK);
    let json = into_text(res).await;
    let result: WebRecordRemovalResult = serde_json::from_str(&json).unwrap();
    assert!(result.record_removed);
    assert!(!result.content_removed);
}

#[test(tokio::test)]
async fn test_get_recording_record_stream() {
    // non-existent record
    let res = get("/api/recording/records/not-found/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // finished
    let res = get("/api/recording/records/finished/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert_matches!(res.headers().get(X_MIRAKURUN_TUNER_USER_ID), Some(_));
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // finished, w/ seekable filters
    let res = get("/api/recording/records/finished/stream?pre-filters[]=cat").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // finished, w/ non-seekable filters
    let res = get("/api/recording/records/finished/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });

    // finished, w/ range covering the entire content
    let res = get_with_test_config(
        "/api/recording/records/finished/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=0-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // finished, w/ seekable filters, w/ range
    let res = get_with_test_config(
        "/api/recording/records/finished/stream?pre-filters[]=cat",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "3");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 1-3/10");
    });
    let content = into_text(res).await;
    assert_eq!(content, "123");

    // finished, w/ non-seekable filters, w/ range
    let res = get_with_test_config(
        "/api/recording/records/finished/stream?post-filters[]=mp4",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // recording
    let res = get("/api/recording/records/recording/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert_matches!(res.headers().get(X_MIRAKURUN_TUNER_USER_ID), Some(_));
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // recording, w/ seekable filters
    let res = get("/api/recording/records/recording/stream?pre-filters[]=cat").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // recording, w/ non-seekable filters
    let res = get("/api/recording/records/recording/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // recording, w/ range covering the entire content
    let res = get_with_test_config(
        "/api/recording/records/recording/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=0-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 0-9/*");
    });
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // recording, w/ seekable filters, w/ range
    let res = get_with_test_config(
        "/api/recording/records/recording/stream?pre-filters[]=cat",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "3");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 1-3/*");
    });
    let content = into_text(res).await;
    assert_eq!(content, "123");

    // recording, w/ non-seekable filters, w/ range
    let res = get_with_test_config(
        "/api/recording/records/finished/stream?post-filters[]=mp4",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    let content = into_text(res).await;
    assert_eq!(content, "0123456789");

    // no content
    let res = get("/api/recording/records/no-content/stream").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // out of range
    let res = get_with_test_config(
        "/api/recording/records/finished/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=100-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    // multiple ranges
    let res = get_with_test_config(
        "/api/recording/records/finished/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3,7-9")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test(tokio::test)]
async fn test_head_recording_record_stream() {
    // non-existent record
    let res = head("/api/recording/records/not-found/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // finished
    let res = head("/api/recording/records/finished/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert_matches!(res.headers().get(X_MIRAKURUN_TUNER_USER_ID), Some(_));

    // finished, w/ seekable filters
    let res = head("/api/recording/records/finished/stream?pre-filters[]=cat").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });

    // finished, w/ non-seekable filters
    let res = head("/api/recording/records/finished/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });

    // finished, w/ range covering the entire content
    let res = head_with_test_config(
        "/api/recording/records/finished/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=0-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), None);

    // finished, w/ seekable filters, w/ range
    let res = head_with_test_config(
        "/api/recording/records/finished/stream?pre-filters[]=cat",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "3");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 1-3/10");
    });

    // finished, w/ non-seekable filters, w/ range
    let res = head_with_test_config(
        "/api/recording/records/finished/stream?post-filters[]=mp4",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);

    // recording
    let res = head("/api/recording/records/recording/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert_matches!(res.headers().get(X_MIRAKURUN_TUNER_USER_ID), Some(_));

    // recording, w/ seekable filters
    let res = head("/api/recording/records/recording/stream?pre-filters[]=cat").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });

    // recording, w/ non-seekable filters
    let res = head("/api/recording/records/recording/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });

    // recording, w/ range covering the entire content
    let res = head_with_test_config(
        "/api/recording/records/recording/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=0-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 0-9/*");
    });

    // recording, w/ seekable filters, w/ range
    let res = head_with_test_config(
        "/api/recording/records/recording/stream?pre-filters[]=cat",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "3");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 1-3/*");
    });

    // recording, w/ non-seekable filters, w/ range
    let res = head_with_test_config(
        "/api/recording/records/finished/stream?post-filters[]=mp4",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);

    // no content
    let res = head("/api/recording/records/no-content/stream").await;
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // out of range
    let res = head_with_test_config(
        "/api/recording/records/finished/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=100-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    // multiple ranges
    let res = head_with_test_config(
        "/api/recording/records/finished/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3,7-9")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test(tokio::test)]
async fn test_get_timeshift_recorders() {
    let res = get("/api/timeshift").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_timeshift_recorder() {
    let res = get("/api/timeshift/test").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/timeshift/not_found").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_timeshift_records() {
    let res = get("/api/timeshift/test/records").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_timeshift_record() {
    let res = get("/api/timeshift/test/records/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/timeshift/test/records/2").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_timeshift_stream() {
    let res = get("/api/timeshift/test/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    let res = get("/api/timeshift/not_found/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // w/ record query

    let res = get("/api/timeshift/test/stream?record=0").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    let res = get("/api/timeshift/test/stream?record=2").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_timeshift_record_stream() {
    // recording
    let res = get("/api/timeshift/test/records/0/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    // recording, w/ seekable filters
    let res = get("/api/timeshift/test/records/0/stream?pre-filters[]=cat").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });

    // recording, w/ non-seekable filters
    let res = get("/api/timeshift/test/records/0/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });

    // recording, w/ range covering the entire content
    let res = get_with_test_config(
        "/api/timeshift/test/records/0/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=0-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 0-9/*");
    });

    // recording, w/ seekable filters, w/ range
    let res = get_with_test_config(
        "/api/timeshift/test/records/0/stream?pre-filters[]=cat",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "3");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 1-3/*");
    });

    // recording, w/ non-seekable filters, w/ range
    let res = get_with_test_config(
        "/api/timeshift/test/records/0/stream?post-filters[]=mp4",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);

    // finished
    let res = get("/api/timeshift/test/records/1/stream").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });
    assert!(res.headers().contains_key(X_MIRAKURUN_TUNER_USER_ID));

    // finished, w/ seekable filters
    let res = get("/api/timeshift/test/records/1/stream?pre-filters[]=cat").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/MP2T");
    });

    // finished, w/ non-seekable filters
    let res = get("/api/timeshift/test/records/1/stream?post-filters[]=mp4").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "video/mp4");
    });

    // finished, w/ range covering the entire content
    let res = get_with_test_config(
        "/api/timeshift/test/records/1/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=0-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "10");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), None);

    // finished, w/ seekable filters, w/ range
    let res = get_with_test_config(
        "/api/timeshift/test/records/1/stream?pre-filters[]=cat",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "bytes");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), Some(v) => {
        assert_eq!(v, "3");
    });
    assert_matches!(res.headers().get(CONTENT_RANGE), Some(v) => {
        assert_eq!(v, "bytes 1-3/10");
    });

    // finished, w/ non-seekable filters, w/ range
    let res = get_with_test_config(
        "/api/timeshift/test/records/1/stream?post-filters[]=mp4",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(ACCEPT_RANGES), Some(v) => {
        assert_eq!(v, "none");
    });
    assert_matches!(res.headers().get(CONTENT_LENGTH), None);
    assert_matches!(res.headers().get(CONTENT_RANGE), None);

    // non-existent record
    let res = get("/api/timeshift/test/records/2/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // non-existent recorder
    let res = get("/api/timeshift/not_found/records/0/stream").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // out of range
    let res = get_with_test_config(
        "/api/timeshift/test/records/1/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=100-")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    // multiple ranges
    let res = get_with_test_config(
        "/api/timeshift/test/records/1/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([("range", "bytes=1-3,7-9")]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[test(tokio::test)]
async fn test_get_iptv_playlist() {
    test_get_iptv_playlist_("/api/iptv/playlist").await;
}

#[test(tokio::test)]
async fn test_get_iptv_channel_m3u8() {
    test_get_iptv_playlist_("/api/iptv/channel.m3u8").await;
}

async fn test_get_iptv_playlist_(endpoint: &str) {
    let res = get(endpoint).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "application/x-mpegurl; charset=UTF-8");
    });
    let playlist = into_text(res).await;
    assert_eq!(
        playlist,
        r#"#EXTM3U
#KODIPROP:mimetype=video/mp2t
#EXTINF:-1 tvg-id="1" tvg-logo="http://mirakc:40772/api/services/1/logo" group-title="GR", test
http://mirakc:40772/api/services/1/stream?decode=true
"#
    );

    let res = get(&format!("{endpoint}?post-filters[]=mp4")).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "application/x-mpegurl; charset=UTF-8");
    });
    let playlist = into_text(res).await;
    assert_eq!(
        playlist,
        r#"#EXTM3U
#KODIPROP:mimetype=video/mp4
#EXTINF:-1 tvg-id="1" tvg-logo="http://mirakc:40772/api/services/1/logo" group-title="GR", test
http://mirakc:40772/api/services/1/stream?decode=true&post-filters[0]=mp4
"#
    );
}

#[test(tokio::test)]
async fn test_get_iptv_epg() {
    test_get_iptv_xmltv_("/api/iptv/epg").await;
}

#[test(tokio::test)]
async fn test_get_iptv_xmltv() {
    test_get_iptv_xmltv_("/api/iptv/xmltv").await;
}

async fn test_get_iptv_xmltv_(endpoint: &str) {
    let version = env!("CARGO_PKG_VERSION");

    let res = get(endpoint).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONTENT_TYPE), Some(v) => {
        assert_eq!(v, "application/xml; charset=UTF-8");
    });
    let xmltv = into_text(res).await;
    assert_eq!(
        xmltv,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" ?><!DOCTYPE tv SYSTEM "xmltv.dtd"><tv generator-info-name="mirakc/{version}"><channel id="1"><display-name lang="ja">test</display-name><icon src="http://mirakc:40772/api/services/1/logo" /></channel></tv>"#
        )
    );
}

#[test(tokio::test)]
async fn test_list_onair() {
    let res = get("/api/onair").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_onair() {
    let res = get("/api/onair/1").await;
    assert_eq!(res.status(), StatusCode::OK);

    let res = get("/api/onair/0").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[test(tokio::test)]
async fn test_get_docs() {
    let res = get("/api/docs").await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_get_events() {
    let res = get("/events").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), Some(v) => {
        assert_eq!(v, "close");
    });
    assert_matches!(res.headers().get(CACHE_CONTROL), Some(v) => {
        assert_eq!(v, "no-store");
    });
}

#[test(tokio::test)]
async fn test_x_mirakurun_priority() {
    // Default priority.
    let res = get_with_test_config(
        "/api/channels/GR/ch/stream",
        maplit::hashmap! {
            "tuner_user_priority" => to_json!(TunerUserPriority::from(0)),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    // Positive priority.
    let res = get_with_test_config(
        "/api/channels/GR/ch/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([(X_MIRAKURUN_PRIORITY, "1")]),
            "tuner_user_priority" => to_json!(TunerUserPriority::from(1)),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    // When multiple priorities are specified, the highest one is used.
    let res = get_with_test_config(
        "/api/channels/GR/ch/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([
                (X_MIRAKURUN_PRIORITY, "1"),
                (X_MIRAKURUN_PRIORITY, "2"),
            ]),
            "tuner_user_priority" => to_json!(TunerUserPriority::from(2))
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    // The maximum priority is 128.
    let res = get_with_test_config(
        "/api/channels/GR/ch/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([(X_MIRAKURUN_PRIORITY, "256")]),
            "tuner_user_priority" => to_json!(TunerUserPriority::from(128)),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);

    // Every negative priority is treated as priority(0).
    let res = get_with_test_config(
        "/api/channels/GR/ch/stream",
        maplit::hashmap! {
            "request_headers" => to_json!([(X_MIRAKURUN_PRIORITY, "-2")]),
            "tuner_user_priority" => to_json!(TunerUserPriority::from(0)),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_access_control_private_addr() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "peer_info" => "192.168.0.1:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_access_control_public_addr() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "peer_info" => "8.8.8.8:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[test(tokio::test)]
async fn test_access_control_uds() {
    // TODO: no way to create tokio::net::unix::UCred.
}

#[test(tokio::test)]
async fn test_access_control_no_peer_info() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "peer_info" => "".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test(tokio::test)]
async fn test_access_control_multiple_forwarded_headers() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "request_headers" => to_json!([
                ("Forwarded", "for=192.0.2.43, for=198.51.100.17"),
                ("Forwarded", "for=192.0.2.60;proto=http;by=203.0.113.43"),
            ]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[test(tokio::test)]
async fn test_access_control_multiple_x_forwarded_host_headers() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "request_headers" => to_json!([
                ("X-Forwarded-Host", "a.example"),
                ("X-Forwarded-Host", "b.example"),
            ]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[test(tokio::test)]
async fn test_access_control_multiple_host_headers() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "request_headers" => to_json!([
                ("Host", "a.example"),
                ("Host", "b.example"),
            ]),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[test(tokio::test)]
async fn test_access_control_allowed_host() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "allowed_hosts" => "['mirakc:40772']".to_string(),
            "peer_info" => "127.0.0.1:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_access_control_not_allowed_host() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "allowed_hosts" => "['mirakc:40772']".to_string(),
            "host" => "a.example:40772".to_string(),
            "peer_info" => "127.0.0.1:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[test(tokio::test)]
async fn test_access_control_localhost() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "allowed_hosts" => "['mirakc:40772']".to_string(),
            "host" => "localhost:40772".to_string(),
            "peer_info" => "127.0.0.1:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_access_control_ipv4_loopback() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "allowed_hosts" => "[allowed.host]".to_string(),
            "host" => "127.0.0.1:40772".to_string(),
            "peer_info" => "127.0.0.1:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_access_control_ipv6_loopback() {
    let res = get_with_test_config(
        "/api/version",
        maplit::hashmap! {
            "allowed_hosts" => "[allowed.host]".to_string(),
            "host" => "[::1]:40772".to_string(),
            "peer_info" => "[::1]:10000".to_string(),
        },
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[test(tokio::test)]
async fn test_mount() {
    let res = get("/").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/src").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = get("/src/Cargo.toml").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), None);
    assert_matches!(res.headers().get(CACHE_CONTROL), None);
    let res = get("/src/no-such-file").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/src-with-index").await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_matches!(res.headers().get(LOCATION), Some(v) => {
        assert_eq!(v, "/src-with-index/Cargo.toml");
    });
    let res = get("/src-with-index/Cargo.toml").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), None);
    assert_matches!(res.headers().get(CACHE_CONTROL), None);
    let res = get("/src-with-index/no-such-file").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/src-with-listing").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), None);
    assert_matches!(res.headers().get(CACHE_CONTROL), None);
    let res = get("/src-with-listing/Cargo.toml").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), None);
    assert_matches!(res.headers().get(CACHE_CONTROL), None);
    let res = get("/src-with-listing/no-such-file").await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    let res = get("/Cargo.toml").await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_matches!(res.headers().get(CONNECTION), None);
    assert_matches!(res.headers().get(CACHE_CONTROL), None);
}

#[test(tokio::test)]
async fn test_filter_setting() {
    async fn do_test<H, F>(query: &str, handler: H) -> StatusCode
    where
        H: FnOnce(Qs<FilterSetting>) -> F + Clone + Send + Sync + 'static,
        F: Future<Output = ()> + Send,
    {
        let endpoint = format!("/?{query}");
        let app = Router::new().route("/", routing::get(handler));
        let req = Request::get(endpoint).body(Body::empty()).unwrap();
        app.oneshot(req).await.unwrap().status()
    }

    assert_eq!(
        do_test("", |Qs(v)| async move {
            assert!(v.decode);
            assert!(v.pre_filters.is_empty());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("unknown=0", |Qs(v)| async move {
            assert!(v.decode);
            assert!(v.pre_filters.is_empty());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("decode=0", |Qs(v)| async move {
            assert!(!v.decode);
            assert!(v.pre_filters.is_empty());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("decode=1", |Qs(v)| async move {
            assert!(v.decode);
            assert!(v.pre_filters.is_empty());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("decode=false", |Qs(v)| async move {
            assert!(!v.decode);
            assert!(v.pre_filters.is_empty());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("decode=true", |Qs(v)| async move {
            assert!(v.decode);
            assert!(v.pre_filters.is_empty());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("decode=x", |_| async move {
            unreachable!();
        })
        .await,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        do_test("pre-filters[]=a", |Qs(v)| async move {
            assert!(v.decode);
            assert_eq!(v.pre_filters.len(), 1);
            assert_eq!(v.pre_filters[0], "a".to_string());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("pre-filters[]=a&pre-filters[]=b", |Qs(v)| async move {
            assert!(v.decode);
            assert_eq!(v.pre_filters.len(), 2);
            assert_eq!(v.pre_filters[0], "a".to_string());
            assert_eq!(v.pre_filters[1], "b".to_string());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("pre-filters[0]=a", |Qs(v)| async move {
            assert!(v.decode);
            assert_eq!(v.pre_filters.len(), 1);
            assert_eq!(v.pre_filters[0], "a".to_string());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("pre-filters[0]=a&pre-filters[1]=b", |Qs(v)| async move {
            assert!(v.decode);
            assert_eq!(v.pre_filters.len(), 2);
            assert_eq!(v.pre_filters[0], "a".to_string());
            assert_eq!(v.pre_filters[1], "b".to_string());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("pre-filters[1]=a", |_| async {
            unreachable!();
        })
        .await,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        do_test("pre-filters[1]=a&pre-filters[2]=b", |_| async {
            unreachable!();
        })
        .await,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        do_test("pre-filters=a", |Qs(v)| async move {
            assert!(v.decode);
            assert_eq!(v.pre_filters.len(), 1);
            assert_eq!(v.pre_filters[0], "a".to_string());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test("pre-filters[x]=a", |_| async {
            unreachable!();
        })
        .await,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        do_test("pre-filters[0]=a&pre-filters[0]=b", |Qs(v)| async move {
            assert!(v.decode);
            assert_eq!(v.pre_filters.len(), 1);
            assert_eq!(v.pre_filters[0], "b".to_string());
            assert!(v.post_filters.is_empty());
        })
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test(
            "decode=1&pre-filters[]=a&post-filters[]=b",
            |Qs(v)| async move {
                assert!(v.decode);
                assert_eq!(v.pre_filters.len(), 1);
                assert_eq!(v.pre_filters[0], "a".to_string());
                assert_eq!(v.post_filters.len(), 1);
                assert_eq!(v.post_filters[0], "b".to_string());
            }
        )
        .await,
        StatusCode::OK
    );

    assert_eq!(
        do_test(
            "pre-filters[]=a&decode=1&post-filters[]=b",
            |Qs(v)| async move {
                assert!(v.decode);
                assert_eq!(v.pre_filters.len(), 1);
                assert_eq!(v.pre_filters[0], "a".to_string());
                assert_eq!(v.post_filters.len(), 1);
                assert_eq!(v.post_filters[0], "b".to_string());
            }
        )
        .await,
        StatusCode::OK
    );
}

async fn get(endpoint: &str) -> Response {
    let app = create_app(&Default::default());
    // The axum::extract::Host requires an HTTP Host request header for tests to work properly.
    let req = Request::get(endpoint)
        .header(HOST, "mirakc:40772")
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn get_with_test_config(
    endpoint: &str,
    test_config: HashMap<&'static str, String>,
) -> Response {
    let app = create_app(&test_config);
    // The axum_extract::Host requires an HTTP Host request header for tests to work properly.
    let mut builder = Request::get(endpoint).header(
        HOST,
        if let Some(host) = test_config.get("host") {
            host
        } else {
            "mirakc:40772"
        },
    );
    if let Some(json) = test_config.get("request_headers") {
        let headers: Vec<(String, String)> = serde_json::from_str(json).unwrap();
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
    }
    let req = builder.body(Body::empty()).unwrap();
    app.oneshot(req).await.unwrap()
}

async fn head(endpoint: &str) -> Response {
    let app = create_app(&Default::default());
    // The axum::extract::Host requires an HTTP Host request header for tests to work properly.
    let req = Request::head(endpoint)
        .header(HOST, "mirakc:40772")
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn head_with_test_config(
    endpoint: &str,
    test_config: HashMap<&'static str, String>,
) -> Response {
    let app = create_app(&test_config);
    // The axum_extract::Host requires an HTTP Host request header for tests to work properly.
    let mut builder = Request::get(endpoint).header(
        HOST,
        if let Some(host) = test_config.get("host") {
            host
        } else {
            "mirakc:40772"
        },
    );
    if let Some(json) = test_config.get("request_headers") {
        let headers: Vec<(String, String)> = serde_json::from_str(json).unwrap();
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
    }
    let req = builder.body(Body::empty()).unwrap();
    app.oneshot(req).await.unwrap()
}

async fn post<T>(endpoint: &str, data: T) -> Response
where
    T: serde::Serialize,
{
    let app = create_app(&Default::default());
    // The axum_extract::Host requires an HTTP Host request header for tests to work properly.
    let req = Request::post(endpoint)
        .header(HOST, "mirakc:40772")
        .header(CONTENT_TYPE, APPLICATION_JSON.as_ref())
        .body(Body::from(serde_json::to_vec(&json!(data)).unwrap()))
        .unwrap();
    app.oneshot(req).await.unwrap()
}

async fn delete(endpoint: &str) -> Response {
    let app = create_app(&Default::default());
    // The axum_extract::Host requires an HTTP Host request header for tests to work properly.
    let req = Request::delete(endpoint)
        .header(HOST, "mirakc:40772")
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

fn create_app(test_config: &HashMap<&'static str, String>) -> Router {
    let peer_info = match test_config.get("peer_info") {
        Some(addr) if addr.is_empty() => None,
        Some(addr) => Some(PeerInfo::Tcp {
            addr: addr.parse().unwrap(),
        }),
        None => Some(PeerInfo::Test),
    };
    let config = config_for_test(test_config);
    build_app(config.clone())
        .layer(helper::ReplaceConnectInfoLayer::new(peer_info))
        .with_state(Arc::new(AppState {
            config,
            string_table: string_table_for_test(),
            tuner_manager: TunerManagerStub::new(test_config),
            epg: EpgStub,
            recording_manager: RecordingManagerStub,
            timeshift_manager: TimeshiftManagerStub,
            onair_manager: OnairProgramManagerStub,
            spawner: actlet::stubs::Context::default(),
        }))
}

fn config_for_test(test_config: &HashMap<&'static str, String>) -> Arc<Config> {
    let config_yaml = format!(
        include_str!("config.yml"),
        manifest_dir = env!("CARGO_MANIFEST_DIR"),
        allowed_hosts = test_config.get("allowed_hosts").map_or("", |s| s.as_str()),
    );

    Arc::new(
        serde_norway::from_str::<Config>(&config_yaml)
            .unwrap()
            .normalize(),
    )
}

fn string_table_for_test() -> Arc<StringTable> {
    crate::string_table::load(
        format!("{}/../resources/strings.yml", env!("CARGO_MANIFEST_DIR")).as_str(),
    )
}

async fn into_text(res: Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).to_string()
}

mod helper {
    use std::task::Context;
    use std::task::Poll;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use axum::response::Response;
    use futures::future::BoxFuture;
    use tower::Layer;
    use tower::Service;

    use super::PeerInfo;

    #[derive(Clone)]
    pub(super) struct ReplaceConnectInfoLayer(Option<PeerInfo>);

    impl ReplaceConnectInfoLayer {
        pub(super) fn new(peer_info: Option<PeerInfo>) -> Self {
            ReplaceConnectInfoLayer(peer_info)
        }
    }

    impl<S> Layer<S> for ReplaceConnectInfoLayer {
        type Service = ReplaceConnectInfoService<S>;

        fn layer(&self, inner: S) -> Self::Service {
            ReplaceConnectInfoService {
                inner,
                peer_info: self.0.clone(),
            }
        }
    }

    #[derive(Clone)]
    pub(super) struct ReplaceConnectInfoService<S> {
        inner: S,
        peer_info: Option<PeerInfo>,
    }

    impl<S> Service<Request<Body>> for ReplaceConnectInfoService<S>
    where
        S: Service<Request<Body>, Response = Response> + Send + 'static,
        S::Future: Send + 'static,
    {
        type Response = S::Response;
        type Error = S::Error;
        // `BoxFuture` is a type alias for `Pin<Box<dyn Future + Send + 'a>>`
        type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.inner.poll_ready(cx)
        }

        fn call(&mut self, mut req: Request<Body>) -> Self::Future {
            match self.peer_info {
                Some(ref peer_info) => {
                    req.extensions_mut().insert(ConnectInfo(peer_info.clone()));
                }
                None => {
                    req.extensions_mut().remove::<ConnectInfo<PeerInfo>>();
                }
            }
            Box::pin(self.inner.call(req))
        }
    }
}
