use std::io;
use std::sync::Arc;

use actix_web;
use bytes::Bytes;
use futures;
use serde::{Deserialize, Serialize};
use tokio::stream::Stream;

use crate::chunk_stream::ChunkStream;
use crate::command_util;
use crate::config::Config;
use crate::error::Error;
use crate::epg;
use crate::epg::EpgChannel;
use crate::models::*;
use crate::mpeg_ts_stream::*;
use crate::tuner;

pub async fn serve(config: Arc<Config>) -> Result<(), Error> {
    let server_config = config.server.clone();
    actix_web::HttpServer::new(
        move || {
            actix_web::App::new()
                .data(config.clone())
                .wrap(actix_web::middleware::Logger::default())
                .wrap(actix_web::middleware::DefaultHeaders::new()
                      .header("Server", server_name()))
                .service(create_api_service())
        })
        .bind((server_config.address.as_str(), server_config.port))?
        .keep_alive(0)  // disable keep-alive
        .workers(server_config.workers)
        .run()
        .await?;
    Ok(())
}

fn server_name() -> String {
    format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

// rest api

const CHUNK_SIZE: usize = 4096 * 8;

type ApiResult = Result<actix_web::HttpResponse, Error>;

#[derive(Serialize)]
struct ErrorBody {
    pub code: u16,
    pub reason: Option<&'static str>,
    pub errors: Vec<u8>,
}

impl actix_web::ResponseError for Error {
    fn error_response(&self) -> actix_web::HttpResponse {
        match *self {
            Error::TunerUnavailable =>
                actix_web::HttpResponse::NotFound().json(ErrorBody {
                    code: actix_web::http::StatusCode::NOT_FOUND.as_u16(),
                    reason: None,
                    errors: Vec::new(),
                }),
            Error::ChannelNotFound =>
                actix_web::HttpResponse::NotFound().json(ErrorBody {
                    code: actix_web::http::StatusCode::NOT_FOUND.as_u16(),
                    reason: None,
                    errors: Vec::new(),
                }),
            Error::ServiceNotFound =>
                actix_web::HttpResponse::NotFound().json(ErrorBody {
                    code: actix_web::http::StatusCode::NOT_FOUND.as_u16(),
                    reason: None,
                    errors: Vec::new(),
                }),
            Error::ProgramNotFound =>
                actix_web::HttpResponse::NotFound().json(ErrorBody {
                    code: actix_web::http::StatusCode::NOT_FOUND.as_u16(),
                    reason: None,
                    errors: Vec::new(),
                }),
            _ =>
                actix_web::HttpResponse::InternalServerError().json(ErrorBody {
                    code: actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
                        .as_u16(),
                    reason: None,
                    errors: Vec::new(),
                }),
        }
    }
}

fn create_api_service() -> impl actix_web::dev::HttpServiceFactory {
    actix_web::web::scope("/api")
        .service(get_version)
        .service(get_status)
        .service(get_channels)
        .service(get_services)
        .service(get_service)
        .service(get_programs)
        .service(get_program)
        .service(get_tuners)
        .service(get_channel_stream)
        .service(get_channel_service_stream)
        .service(get_service_stream)
        .service(get_program_stream)
}

#[actix_web::get("/version")]
async fn get_version() -> impl actix_web::Responder {
    actix_web::HttpResponse::Ok().json(env!("CARGO_PKG_VERSION"))
}

#[actix_web::get("/status")]
async fn get_status() -> impl actix_web::Responder {
    actix_web::HttpResponse::Ok().content_type("application/json").body("{}")
}

#[actix_web::get("/channels")]
async fn get_channels() -> ApiResult {
    epg::query_channels().await
        .map(|channels| actix_web::HttpResponse::Ok().json(channels))
}

#[actix_web::get("/services")]
async fn get_services() -> ApiResult {
    epg::query_services().await
        .map(|services| services.into_iter()
             .map(MirakurunService::from).collect::<Vec<MirakurunService>>())
        .map(|services| actix_web::HttpResponse::Ok().json(services))
}

#[actix_web::get("/services/{id}")]
async fn get_service(path: actix_web::web::Path<ServicePath>) -> ApiResult {
    epg::query_service_by_nid_sid(path.id.nid(), path.id.sid()).await
        .map(|service| MirakurunService::from(service))
        .map(|service| actix_web::HttpResponse::Ok().json(service))
}

#[actix_web::get("/programs")]
async fn get_programs() -> ApiResult {
    epg::query_programs().await
        .map(|programs| actix_web::HttpResponse::Ok().json(programs))
}

#[actix_web::get("/programs/{id}")]
async fn get_program(path: actix_web::web::Path<ProgramPath>) -> ApiResult {
    epg::query_program_by_nid_sid_eid(
        path.id.nid(), path.id.sid(), path.id.eid()).await
        .map(|program| actix_web::HttpResponse::Ok().json(program))
}

#[actix_web::get("/tuners")]
async fn get_tuners() -> ApiResult {
    tuner::query_tuners().await
        .map(|tuners| actix_web::HttpResponse::Ok().json(tuners))
}

#[actix_web::get("/channels/{channel_type}/{channel}/stream")]
async fn get_channel_stream(
    config: actix_web::web::Data<Arc<Config>>,
    path: actix_web::web::Path<ChannelPath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let channel = epg::query_channel(
        path.channel_type, path.channel.clone()).await?;

    let filters = make_filters(
        &config, &channel, None, None, "".to_string(),
        query.pre_filter_required(), query.post_filter_required())?;

    let stream = tuner::start_streaming(
        path.channel_type, path.channel.clone(), user).await?;

    streaming(stream, filters)
}

#[actix_web::get("/channels/{channel_type}/{channel}/services/{sid}/stream")]
async fn get_channel_service_stream(
    config: actix_web::web::Data<Arc<Config>>,
    path: actix_web::web::Path<ChannelServicePath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let channel = epg::query_channel(
        path.channel_type, path.channel.clone()).await?;

    do_get_service_stream(config, &channel, path.sid, query, user).await
}

#[actix_web::get("/services/{id}/stream")]
async fn get_service_stream(
    config: actix_web::web::Data<Arc<Config>>,
    path: actix_web::web::Path<ServicePath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let service = epg::query_service_by_nid_sid(
        path.id.nid(), path.id.sid()).await?;
    do_get_service_stream(
        config, &service.channel, service.sid, query, user).await
}

#[actix_web::get("/programs/{id}/stream")]
async fn get_program_stream(
    config: actix_web::web::Data<Arc<Config>>,
    path: actix_web::web::Path<ProgramPath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let program = epg::query_program_by_nid_sid_eid(
        path.id.nid(), path.id.sid(), path.id.eid()).await?;
    let service = epg::query_service_by_nid_sid(
        path.id.nid(), path.id.sid()).await?;
    let clock = epg::query_clock(service.triple()).await?;

    let ch_type = service.channel.channel_type;
    let ch = service.channel.channel.clone();

    let filters = make_program_filters(
        &config, &service.channel, &program, &clock,
        query.pre_filter_required(), query.post_filter_required())?;

    let stream = tuner::start_streaming(ch_type, ch, user).await?;

    streaming(stream, filters)
}

async fn do_get_service_stream(
    config: actix_web::web::Data<Arc<Config>>,
    channel: &EpgChannel,
    sid: ServiceId,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let filters = make_service_filters(
        &config, channel, sid,
        query.pre_filter_required(), query.post_filter_required())?;

    let stream = tuner::start_streaming(
        channel.channel_type, channel.channel.clone(), user).await?;

    streaming(stream, filters)
}

fn make_service_filters(
    config: &Config,
    channel: &EpgChannel,
    sid: ServiceId,
    pre_filter_required: bool,
    post_filter_required: bool,
) -> Result<Vec<String>, Error> {
    let filter = make_service_filter_command(
        &config.filters.service_filter, sid)?;
    make_filters(
        config, channel, Some(sid), None,
        filter, pre_filter_required, post_filter_required)
}

fn make_program_filters(
    config: &Config,
    channel: &EpgChannel,
    program: &MirakurunProgram,
    clock: &Clock,
    pre_filter_required: bool,
    post_filter_required: bool,
) -> Result<Vec<String>, Error> {
    let filter = make_program_filter_command(
        &config.filters.program_filter, program.service_id, program.event_id,
        clock)?;
    make_filters(
        config, channel, Some(program.service_id), Some(program.event_id),
        filter, pre_filter_required, post_filter_required)
}

fn make_filters(
    config: &Config,
    channel: &EpgChannel,
    sid: Option<ServiceId>,
    eid: Option<EventId>,
    filter: String,
    pre_filter_required: bool,
    post_filter_required: bool,
) -> Result<Vec<String>, Error> {
    let mut filters = Vec::new();

    if pre_filter_required {
        if config.filters.pre_filter.is_empty() {
            log::warn!("Pre-filter is required, but not defined");
        } else {
            let cmd = make_filter_command(
                &config.filters.pre_filter, channel, sid, eid)?;
            filters.push(cmd);
        }
    }

    if filter.is_empty() {
        log::warn!("Filter not defined");
    } else {
        filters.push(filter);
    }

    if post_filter_required {
        if config.filters.post_filter.is_empty() {
            log::warn!("Post-filter is required, but not defined");
        } else {
            let cmd = make_filter_command(
                &config.filters.post_filter, channel, sid, eid)?;
            filters.push(cmd);
        }
    }

    Ok(filters)
}

fn make_filter_command(
    command: &str,
    channel:  &EpgChannel,
    sid: Option<ServiceId>,
    eid: Option<EventId>,
) -> Result<String, Error> {
    let template = mustache::compile_str(command)?;
    let mut builder = mustache::MapBuilder::new();
    builder = builder.insert("channel_type", &channel.channel_type)?;
    builder = builder.insert_str("xsids", &channel.channel);
    if let Some(sid) = sid {
        builder = builder.insert_str("sid", sid.value().to_string());
    }
    if let Some(eid) = eid {
        builder = builder.insert_str("eid", eid.value().to_string());
    }
    let data = builder.build();
    Ok(template.render_data_to_string(&data)?)
}

fn streaming(stream: MpegTsStream, filters: Vec<String>) -> ApiResult {
    if filters.is_empty() {
        do_streaming(stream.id(), stream)
    } else {
        let stream_id = stream.id();
        let (input, output) = command_util::spawn_pipeline(
            filters, stream_id)?;
        actix::spawn(stream.pipe(input));
        do_streaming(stream_id, ChunkStream::new(output, CHUNK_SIZE))
    }
}

fn do_streaming<S>(id: MpegTsStreamId, stream: S) -> ApiResult
where
    S: Stream<Item = io::Result<Bytes>> + Unpin + 'static,
{
    Ok(actix_web::HttpResponse::Ok()
       .force_close()
       .set_header("cache-control", "no-store")
       .set_header("content-type", "video/MP2T")
       .streaming(MpegTsStreamTerminator::new(id, stream)))
}

// extractors

#[derive(Deserialize)]
struct ChannelPath {
    channel_type: ChannelType,
    channel: String,
}

#[derive(Deserialize)]
struct ChannelServicePath {
    channel_type: ChannelType,
    channel: String,
    sid: ServiceId,
}

#[derive(Deserialize)]
struct ServicePath {
    id: MirakurunServiceId,
}

#[derive(Deserialize)]
struct ProgramPath {
    id: MirakurunProgramId,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct StreamQuery {
    #[serde(default)]
    pre_filter: Option<bool>,
    #[serde(default)]
    post_filter: Option<bool>,

    // For compatibility with Mirakurun.
    // The post-filter parameter can override the decode parameter.
    #[serde(default)]
    decode: u8,  // default: 0
}

impl StreamQuery {
    fn pre_filter_required(&self) -> bool {
        match self.pre_filter {
            Some(pre_filter) => pre_filter,
            None => false,
        }
    }

    fn post_filter_required(&self) -> bool {
        match (self.post_filter, self.decode) {
            (Some(post_filter), _) => post_filter,
            (None, decode) => decode != 0,  // for compatibility with Mirakurun
        }
    }
}

impl actix_web::FromRequest for TunerUser {
    type Error = actix_web::Error;
    type Future = futures::future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(
        req: &actix_web::HttpRequest,
        _: &mut actix_web::dev::Payload
    ) -> Self::Future {
        let remote = req
            .connection_info()
            .remote()
            .map(|v| v.to_string());

        let agent = req
            .headers()
            .get_all(actix_web::http::header::USER_AGENT)
            .last()
            .map(|value| {
                value.to_str().ok().map_or(String::new(), |s| s.to_string())
            });

        let info = TunerUserInfo::Web { remote, agent };

        let priority = req.headers().get_all("x-mirakurun-priority")
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.parse::<i32>().ok())
            .max()
            .map(|value| value.max(0))
            .map(TunerUserPriority::from)
            .unwrap_or_default();

        futures::future::ok(TunerUser { info, priority })
    }
}

fn make_service_filter_command(
    command: &str,
    sid: ServiceId
) -> Result<String, Error> {
    let template = mustache::compile_str(command)?;
    let data = mustache::MapBuilder::new()
        .insert_str("sid", sid.value().to_string())
        .build();
    Ok(template.render_data_to_string(&data)?)
}

fn make_program_filter_command(
    command: &str,
    sid: ServiceId,
    eid: EventId,
    clock: &Clock,
) -> Result<String, Error> {
    let template = mustache::compile_str(command)?;
    let data = mustache::MapBuilder::new()
        .insert_str("sid", sid.value().to_string())
        .insert_str("eid", eid.value().to_string())
        .insert_str("clock_pcr", clock.pcr.to_string())
        .insert_str("clock_time", clock.time.to_string())
        .build();
    Ok(template.render_data_to_string(&data)?)
}

// tests

#[cfg(test)]
mod tests {
    use super::*;

    // TODO
    // ----
    // There is no good mocking framework in Rust at this moment.
    //
    // mockall doesn't work with multiple threads.  Especially, constructor
    // methods defined as static methods.
    //
    // mockers doesn't work in stable Rust because it requires nightly features.
    //
    // Mocking frameworks are listed in:
    //
    //   https://asomers.github.io/mock_shootout/
    //

    async fn request(
        method: actix_web::http::Method,
        uri: &str
    ) -> actix_web::HttpResponse {
        let mut config = Config::default();
        // Disable all filters
        config.filters.pre_filter = String::new();
        config.filters.service_filter = String::new();
        config.filters.program_filter = String::new();
        config.filters.post_filter = String::new();

        let mut app = actix_web::test::init_service(
            actix_web::App::new()
                .data(Arc::new(config))
                .service(create_api_service())).await;
        let req = actix_web::test::TestRequest::with_uri(uri)
            .method(method).to_request();
        actix_web::test::call_service(&mut app, req).await.into()
    }

    macro_rules! impl_method {
        ($method:ident, $METHOD:ident) => {
            async fn $method(uri: &str) -> actix_web::HttpResponse {
                request(actix_web::http::Method::$METHOD, uri).await
            }
        }
    }

    impl_method!(get, GET);

    #[actix_rt::test]
    async fn test_get_unknown() {
        let res = get("/api/unknown").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_get_version() {
        let res = get("/api/version").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_get_status() {
        let res = get("/api/status").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_get_channels() {
        let res = get("/api/channels").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_get_programs() {
        let res = get("/api/programs").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_get_program() {
        let res = get("/api/programs/1").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/programs/0").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
    }

    #[actix_rt::test]
    async fn test_get_tuners() {
        let res = get("/api/tuners").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_get_channel_stream() {
        let res = get("/api/channels/GR/ch/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/channels/GR/0/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let res = get("/api/channels/WOWOW/ch/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        let valid_pairs =
            ["GR", "BS", "CS", "SKY"].iter().zip(&decode_values);
        for (channel_type, decode) in valid_pairs {
            let res = get(format!("/api/channels/{}/ch/stream?decode={}",
                                  channel_type, decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!("/api/channels/WOWOW/ch/stream?decode={}",
                                  decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }
    }

    #[actix_rt::test]
    async fn test_get_channel_service_stream() {
        let res = get("/api/channels/GR/ch/services/1/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/channels/GR/0/services/1/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let res = get("/api/channels/WOWOW/ch/services/1/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        let valid_pairs =
            ["GR", "BS", "CS", "SKY"].iter().zip(&decode_values);
        for (channel_type, decode) in valid_pairs {
            let res = get(format!(
                "/api/channels/{}/ch/services/1/stream?decode={}",
                channel_type, decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!(
                "/api/channels/WOWOW/ch/services/1/stream?decode={}",
                decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }
    }

    #[actix_rt::test]
    async fn test_get_service_stream() {
        let res = get("/api/services/1/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/services/0/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        for decode in &decode_values {
            let res = get(format!("/api/services/1/stream?decode={}",
                                  decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!("/api/services/0/stream?decode={}",
                                  decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }
    }

    #[actix_rt::test]
    async fn test_get_program_stream() {
        let res = get("/api/programs/100001/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/programs/0/stream").await;
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        for decode in &decode_values {
            let res = get(format!("/api/programs/100001/stream?decode={}",
                                  decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!("/api/programs/0/stream?decode={}",
                                  decode).as_str()).await;
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }
    }

    #[test]
    fn test_stream_query() {
        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "").unwrap().into_inner();
        assert_eq!(query.pre_filter_required(), false);
        assert_eq!(query.post_filter_required(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "pre-filter=true").unwrap().into_inner();
        assert_eq!(query.pre_filter_required(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "pre-filter=false").unwrap().into_inner();
        assert_eq!(query.pre_filter_required(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "pre-filter");
        assert!(query.is_err());

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "pre-filter=");
        assert!(query.is_err());

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "pre-filter=1");
        assert!(query.is_err());

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "post-filter=true").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "post-filter=false").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "decode=1").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "decode=0").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "decode=2").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "post-filter=true&decode=0").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "post-filter=false&decode=1").unwrap().into_inner();
        assert_eq!(query.post_filter_required(), false);
    }
}
