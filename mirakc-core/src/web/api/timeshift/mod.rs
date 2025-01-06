pub(super) mod records;

use super::*;

use crate::filter::FilterPipelineBuilder;
use crate::models::TunerUser;
use crate::timeshift;
use crate::timeshift::TimeshiftRecorderModel;
use crate::timeshift::TimeshiftRecorderQuery;
use crate::web::api::stream::streaming;
use crate::web::api::stream::StreamingHeaderParams;

/// Lists timeshift recorders.
#[utoipa::path(
    get,
    path = "/timeshift",
    responses(
        (status = 200, description = "OK", body = [WebTimeshiftRecorder]),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getTimeshiftRecorders",
)]
pub(in crate::web::api) async fn list<S>(
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
) -> Result<Json<Vec<WebTimeshiftRecorder>>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecorders>,
{
    timeshift_manager
        .call(timeshift::QueryTimeshiftRecorders)
        .await?
        .map(|recorders| {
            recorders
                .into_iter()
                .map(WebTimeshiftRecorder::from)
                .collect::<Vec<WebTimeshiftRecorder>>()
        })
        .map(Json::from)
}

/// Gets a timeshift recorder.
#[utoipa::path(
    get,
    path = "/timeshift/{recorder}",
    params(
        ("recorder" = String, Path, description = "Timeshift recorder name"),
    ),
    responses(
        (status = 200, description = "OK", body = WebTimeshiftRecorder),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    operation_id = "getTimeshiftRecorder",
)]
pub(in crate::web::api) async fn get<S>(
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    Path(recorder): Path<String>,
) -> Result<Json<WebTimeshiftRecorder>, Error>
where
    S: Call<timeshift::QueryTimeshiftRecorder>,
{
    let msg = timeshift::QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(recorder),
    };
    timeshift_manager
        .call(msg)
        .await?
        .map(WebTimeshiftRecorder::from)
        .map(Json::from)
}

/// Gets a live stream of a timeshift record.
#[utoipa::path(
    get,
    path = "/timeshift/{recorder}/stream",
    params(
        ("recorder" = String, Path, description = "Timeshift recorder name"),
        ("pre-filters" = Option<[String]>, Query, description = "Pre-filters"),
        ("post-filters" = Option<[String]>, Query, description = "post-filters"),
        ("record" = Option<u32>, Query, description = "Timeshift record ID"),
    ),
    responses(
        (status = 200, description = "OK"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Tuner Resource Unavailable"),
    ),
    operation_id = "getTimeshiftRecorderStream",
)]
pub(in crate::web::api) async fn stream<S>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(TimeshiftManagerExtractor(timeshift_manager)): State<TimeshiftManagerExtractor<S>>,
    Path(recorder_id): Path<String>,
    user: TunerUser,
    Qs(filter_setting): Qs<FilterSetting>,
    Query(TimeshiftRecordQuery { record: record_id }): Query<TimeshiftRecordQuery>,
) -> Result<Response, Error>
where
    S: Call<timeshift::CreateTimeshiftLiveStreamSource>,
    S: Call<timeshift::QueryTimeshiftRecorder>,
{
    let msg = timeshift::QueryTimeshiftRecorder {
        recorder: TimeshiftRecorderQuery::ByName(recorder_id.clone()),
    };
    let recorder = timeshift_manager.call(msg).await??;

    let msg = timeshift::CreateTimeshiftLiveStreamSource {
        recorder: TimeshiftRecorderQuery::ByName(recorder_id.clone()),
        record_id,
    };
    let src = timeshift_manager.call(msg).await??;

    let (filters, content_type, _seekable) = build_filters(&config, &filter_setting, &recorder)?;

    let (stream, stop_trigger) = src.create_stream().await?; // TODO: seekable

    let params = StreamingHeaderParams {
        seekable: false, // always non-seekable
        content_type,
        length: None,
        range: None,
        user,
    };

    streaming(&config, stream, filters, &params, stop_trigger).await
}

fn build_filters(
    config: &Config,
    filter_setting: &FilterSetting,
    recorder: &TimeshiftRecorderModel,
) -> Result<(Vec<String>, String, bool), Error> {
    let data = mustache::MapBuilder::new()
        .insert_str("channel_name", &recorder.service.channel.name)
        .insert("channel_type", &recorder.service.channel.channel_type)?
        .insert_str("channel", &recorder.service.channel.channel)
        .insert("sid", &recorder.service.id.sid())?
        .build();

    let mut builder = FilterPipelineBuilder::new(data, true); // seekable by default
    builder.add_pre_filters(&config.pre_filters, &filter_setting.pre_filters)?;
    // The stream has already been decoded.
    builder.add_post_filters(&config.post_filters, &filter_setting.post_filters)?;
    Ok(builder.build())
}
