use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use actix::prelude::*;
use actix_files;
use actix_service;
use actix_web;
use bytes::Bytes;
use futures;
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::airtime_tracker;
use crate::chunk_stream::ChunkStream;
use crate::command_util::*;
use crate::config::{Config, ServerAddr};
use crate::error::Error;
use crate::epg::*;
use crate::models::*;
use crate::mpeg_ts_stream::*;
use crate::tuner::*;

#[cfg(not(test))]
type TunerManagerActor = TunerManager;
#[cfg(test)]
type TunerManagerActor = actix::actors::mocker::Mocker<TunerManager>;

#[cfg(not(test))]
type EpgActor = Epg;
#[cfg(test)]
type EpgActor = actix::actors::mocker::Mocker<Epg>;

pub async fn serve(
    config: Arc<Config>,
    tuner_manager: Addr<TunerManager>,
    epg: Addr<Epg>,
) -> Result<(), Error> {
    let server_config = config.server.clone();
    let mut server = actix_web::HttpServer::new(
        move || {
            actix_web::App::new()
                .data(config.clone())
                .data(tuner_manager.clone())
                .data(epg.clone())
                .wrap(actix_web::middleware::Logger::default())
                .wrap(actix_web::middleware::DefaultHeaders::new()
                      .header("Server", server_name()))
                .wrap(AccessControl)
                .service(create_api_service())
        });
    for addr in server_config.addrs.iter() {
        server = match addr {
            ServerAddr::Http(addr) => server.bind(addr.as_str())?,
            ServerAddr::Unix(path) => server.bind_uds(path.as_str())?,
        };
    }
    server
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
            Error::StreamingTimedOut =>
                actix_web::HttpResponse::RequestTimeout().json(ErrorBody {
                    code: actix_web::http::StatusCode::REQUEST_TIMEOUT.as_u16(),
                    reason: None,
                    errors: Vec::new(),
                }),
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
            Error::AccessDenied =>
                actix_web::HttpResponse::Forbidden().json(ErrorBody {
                    code: actix_web::http::StatusCode::FORBIDDEN.as_u16(),
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
        .service(get_docs)
}

#[actix_web::get("/version")]
async fn get_version() -> ApiResult {
    Ok(actix_web::HttpResponse::Ok().json(env!("CARGO_PKG_VERSION")))
}

#[actix_web::get("/status")]
async fn get_status() -> ApiResult {
    Ok(actix_web::HttpResponse::Ok()
       .content_type("application/json").body("{}"))
}

#[actix_web::get("/channels")]
async fn get_channels(
    epg: actix_web::web::Data<Addr<EpgActor>>,
) -> ApiResult {
    epg.send(QueryChannelsMessage).await?
        .map(|channels| actix_web::HttpResponse::Ok().json(channels))
}

#[actix_web::get("/services")]
async fn get_services(
    epg: actix_web::web::Data<Addr<EpgActor>>,
) -> ApiResult {
    epg.send(QueryServicesMessage).await?
        .map(|services| services.into_iter()
             .map(MirakurunService::from).collect::<Vec<MirakurunService>>())
        .map(|services| actix_web::HttpResponse::Ok().json(services))
}

#[actix_web::get("/services/{id}")]
async fn get_service(
    epg: actix_web::web::Data<Addr<EpgActor>>,
    path: actix_web::web::Path<ServicePath>,
) -> ApiResult {
    epg.send(QueryServiceMessage::ByNidSid {
        nid: path.id.nid(),
        sid: path.id.sid(),
    }).await?
        .map(MirakurunService::from)
        .map(|service| actix_web::HttpResponse::Ok().json(service))
}

#[actix_web::get("/programs")]
async fn get_programs(
    epg: actix_web::web::Data<Addr<EpgActor>>,
) -> ApiResult {
    epg.send(QueryProgramsMessage).await?
        .map(|programs| programs.into_iter()
             .map(MirakurunProgram::from).collect::<Vec<MirakurunProgram>>())
        .map(|programs| actix_web::HttpResponse::Ok().json(programs))
}

#[actix_web::get("/programs/{id}")]
async fn get_program(
    epg: actix_web::web::Data<Addr<EpgActor>>,
    path: actix_web::web::Path<ProgramPath>,
) -> ApiResult {
    epg.send(QueryProgramMessage::ByNidSidEid {
        nid: path.id.nid(),
        sid: path.id.sid(),
        eid: path.id.eid(),
    }).await?
        .map(MirakurunProgram::from)
        .map(|program| actix_web::HttpResponse::Ok().json(program))
}

#[actix_web::get("/tuners")]
async fn get_tuners(
    tuner_manager: actix_web::web::Data<Addr<TunerManagerActor>>
) -> ApiResult {
    tuner_manager.send(QueryTunersMessage).await?
        .map(|tuners| actix_web::HttpResponse::Ok().json(tuners))
}

#[actix_web::get("/channels/{channel_type}/{channel}/stream")]
async fn get_channel_stream(
    config: actix_web::web::Data<Arc<Config>>,
    tuner_manager: actix_web::web::Data<Addr<TunerManagerActor>>,
    epg: actix_web::web::Data<Addr<EpgActor>>,
    path: actix_web::web::Path<ChannelPath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let channel = epg.send(QueryChannelMessage {
        channel_type: path.channel_type,
        channel: path.channel.clone(),
    }).await??;

    let filters = make_filters(
        &config, &channel, None, None, "".to_string(),
        query.pre_filter_required(), query.post_filter_required())?;

    let stream = tuner_manager.send(StartStreamingMessage {
        channel, user
    }).await??;

    streaming(&config, stream, filters, None).await
}

#[actix_web::get("/channels/{channel_type}/{channel}/services/{sid}/stream")]
async fn get_channel_service_stream(
    config: actix_web::web::Data<Arc<Config>>,
    tuner_manager: actix_web::web::Data<Addr<TunerManagerActor>>,
    epg: actix_web::web::Data<Addr<EpgActor>>,
    path: actix_web::web::Path<ChannelServicePath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let channel = epg.send(QueryChannelMessage {
        channel_type: path.channel_type,
        channel: path.channel.clone(),
    }).await??;

    do_get_service_stream(
        config, tuner_manager, channel, path.sid, query, user).await
}

#[actix_web::get("/services/{id}/stream")]
async fn get_service_stream(
    config: actix_web::web::Data<Arc<Config>>,
    tuner_manager: actix_web::web::Data<Addr<TunerManagerActor>>,
    epg: actix_web::web::Data<Addr<EpgActor>>,
    path: actix_web::web::Path<ServicePath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let service = epg.send(QueryServiceMessage::ByNidSid {
        nid: path.id.nid(),
        sid: path.id.sid(),
    }).await??;

    do_get_service_stream(
        config, tuner_manager, service.channel, service.sid, query, user).await
}

#[actix_web::get("/programs/{id}/stream")]
async fn get_program_stream(
    config: actix_web::web::Data<Arc<Config>>,
    tuner_manager: actix_web::web::Data<Addr<TunerManagerActor>>,
    epg: actix_web::web::Data<Addr<EpgActor>>,
    path: actix_web::web::Path<ProgramPath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let program = epg.send(QueryProgramMessage::ByNidSidEid {
        nid: path.id.nid(),
        sid: path.id.sid(),
        eid: path.id.eid(),
    }).await??;

    let service = epg.send(QueryServiceMessage::ByNidSid {
        nid: path.id.nid(),
        sid: path.id.sid(),
    }).await??;

    let clock = epg.send(QueryClockMessage {
        triple: service.triple(),
    }).await??;

    let filters = make_program_filters(
        &config, &service.channel, &program, &clock,
        query.pre_filter_required(), query.post_filter_required())?;

    let stream = tuner_manager.send(StartStreamingMessage {
        channel: service.channel.clone(),
        user
    }).await??;

    let stop_trigger = airtime_tracker::track_airtime(
        &config.recorder.track_airtime_command, &service.channel, &program,
        stream.id(), tuner_manager.get_ref().clone(), epg.get_ref().clone()
    ).await?;

    let result = streaming(&config, stream, filters, stop_trigger).await;

    match result {
        Err(Error::ProgramNotFound) =>
            log::warn!("No stream for the program#{}, maybe canceled", path.id),
        _ => (),
    }

    result
}

#[actix_web::get("/docs")]
async fn get_docs(
    config: actix_web::web::Data<Arc<Config>>,
) -> io::Result<actix_files::NamedFile> {
    // Mirakurun client requires this API since Mirakurun/2.14.0.
    //
    // mirakc simply returns a JSON data obtained from Mirakurun.
    Ok(actix_files::NamedFile::open(&config.mirakurun.openapi_json)?)
}

async fn do_get_service_stream(
    config: actix_web::web::Data<Arc<Config>>,
    tuner_manager: actix_web::web::Data<Addr<TunerManagerActor>>,
    channel: EpgChannel,
    sid: ServiceId,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    let filters = make_service_filters(
        &config, &channel, sid,
        query.pre_filter_required(), query.post_filter_required())?;

    let stream = tuner_manager.send(StartStreamingMessage {
        channel, user
    }).await??;

    streaming(&config, stream, filters, None).await
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
    program: &EpgProgram,
    clock: &Clock,
    pre_filter_required: bool,
    post_filter_required: bool,
) -> Result<Vec<String>, Error> {
    let filter = make_service_filter_command(
        &config.filters.service_filter, program.quad.sid())?;
    let mut filters = make_filters(
        config, channel, Some(program.quad.sid()), Some(program.quad.eid()),
        filter, pre_filter_required, post_filter_required)?;
    let filter = make_program_filter_command(
        &config.filters.program_filter, program.quad.sid(), program.quad.eid(),
        clock)?;
    if filter.is_empty() {
        log::warn!("Filter not defined");
    } else {
        filters.push(filter);
    }
    Ok(filters)
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
    builder = builder.insert_str("channel", &channel.channel);
    if let Some(sid) = sid {
        builder = builder.insert_str("sid", sid.value().to_string());
    }
    if let Some(eid) = eid {
        builder = builder.insert_str("eid", eid.value().to_string());
    }
    let data = builder.build();
    Ok(template.render_data_to_string(&data)?)
}

async fn streaming(
    config: &Config,
    mut stream: MpegTsStream,
    filters: Vec<String>,
    stop_trigger: Option<MpegTsStreamStopTrigger>,
) -> ApiResult {
    if filters.is_empty() {
        do_streaming(stream, config.server.stream_time_limit).await
    } else {
        let stop_trigger2 = stream.take_stop_trigger();

        let mut pipeline = spawn_pipeline(filters, stream.id())?;

        let (input, output) = pipeline.take_endpoints()?;

        let stream_id = stream.id();
        actix::spawn(async {
            let _ = stream.pipe(input).await;
        });

        // Use a MPSC channel as a buffer.
        //
        // The command pipeline often breaks when reading stops for a few
        // seconds.
        let mut stream = ChunkStream::new(
            output, config.server.stream_chunk_size);
        let (mut sender, receiver) =
            mpsc::channel(config.server.stream_max_chunks);
        actix::spawn(async move {
            while let Some(result) = stream.next().await {
                if let Ok(chunk) = result {
                    log::trace!("{}: Received a filtered chunk of {} bytes",
                                stream_id, chunk.len());
                    // The task yields if the buffer is full.
                    if let Err(_) = sender.send(Ok(chunk)).await {
                        log::debug!("{}: Disconnected by client", stream_id);
                        break;
                    }
                } else {
                    log::error!("{}: Error, stop streaming", stream_id);
                    break;
                }

                // Always yield for sending the chunk to the client quickly.
                //
                // The async task never yields voluntarily and can starve other
                // tasks waiting on the same executor.  For avoiding the
                // starvation, the task has to yields within a short term.
                //
                // Theoretically, one 32 KiB chunk comes every 10 ms.  This
                // period is a long enough time in the CPU time point of view.
                // Therefore, the async task simply yields at the end of every
                // iteration.
                tokio::task::yield_now().await;
            }

            drop(pipeline);
        });

        do_streaming(
            MpegTsStreamTerminator::new(receiver, [stop_trigger, stop_trigger2]),
            config.server.stream_time_limit).await
    }
}

async fn do_streaming<S>(stream: S, time_limit: u64) -> ApiResult
where
    // actix_web::dev::HttpResponseBuilder::streaming() requires 'static...
    S: Stream<Item = io::Result<Bytes>> + Unpin + 'static,
{
    // No data is sent to the client until the first TS packet comes from the
    // streaming pipeline.
    let mut peekable = stream.peekable();
    let fut = Pin::new(&mut peekable).peek();
    match tokio::time::timeout(Duration::from_millis(time_limit), fut).await {
        Ok(None) => {
            // No packets come from the pipeline, maybe the program has been
            // canceled.
            Err(Error::ProgramNotFound)
        }
        Err(_) => {
            Err(Error::StreamingTimedOut)
        }
        Ok(_) =>  {
            // Send the response headers and start streaming.
            Ok(actix_web::HttpResponse::Ok()
               .force_close()
               .set_header("cache-control", "no-store")
               .set_header("content-type", "video/MP2T")
               .streaming(peekable.into_inner()))
        }
    }
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

// middleware

struct AccessControl;

impl<S, B> actix_service::Transform<S> for AccessControl
where
    S: actix_service::Service<Request = actix_web::dev::ServiceRequest,
                              Response = actix_web::dev::ServiceResponse<B>,
                              Error = actix_web::Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = actix_web::dev::ServiceRequest;
    type Response = actix_web::dev::ServiceResponse<B>;
    type Error = actix_web::Error;
    type InitError = ();
    type Transform = AccessControlMiddleware<S>;
    type Future =
        futures::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        futures::future::ok(AccessControlMiddleware(service))
    }
}

struct AccessControlMiddleware<S>(S);

impl<S, B> actix_service::Service for AccessControlMiddleware<S>
where
    S: actix_service::Service<Request = actix_web::dev::ServiceRequest,
                              Response = actix_web::dev::ServiceResponse<B>,
                              Error = actix_web::Error>,
    S::Future: 'static,
    B: 'static,
{
    type Request = actix_web::dev::ServiceRequest;
    type Response = actix_web::dev::ServiceResponse<B>;
    type Error = actix_web::Error;
    type Future = Pin<Box<dyn futures::future::Future<
            Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: actix_web::dev::ServiceRequest) -> Self::Future {
        // Take no account of HTTP Forwarded and X-Forwarded-For headers.
        let ip = req.peer_addr().map(|socket| socket.ip());

        let allowed = match ip {
            Some(ip) => is_private_ip_addr(ip),
            None => true,  // UNIX domain socket
        };

        if allowed {
            Box::pin(self.0.call(req))
        } else {
            Box::pin(futures::future::ok(req.error_response(
                actix_web::error::ErrorForbidden(Error::AccessDenied))))
        }
    }
}

fn is_private_ip_addr(ip: IpAddr) -> bool {
    // TODO: IpAddr::is_global() is a nightly-only API at this point.
    match ip {
        IpAddr::V4(ip) => is_private_ipv4_addr(ip),
        IpAddr::V6(ip) => is_private_ipv6_addr(ip),
    }
}

fn is_private_ipv4_addr(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_private_ipv6_addr(ip: Ipv6Addr) -> bool {
    // TODO: Support only IPv4-compatible and IPv4-mapped addresses at this
    //       moment.
    match ip.to_ipv4() {
        Some(ip) => is_private_ipv4_addr(ip),
        None => false,
    }
}

// tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use actix_http;
    use matches::*;
    use crate::broadcaster::BroadcasterStream;

    async fn request(req: actix_http::Request) -> actix_web::HttpResponse {
        let mut app = actix_web::test::init_service(
            actix_web::App::new()
                .data(config_for_test())
                .data(tuner_manager_for_test())
                .data(epg_for_test())
                .wrap(AccessControl)
                .service(create_api_service())).await;
        actix_web::test::call_service(&mut app, req).await.into()
    }

    async fn get(uri: &str) -> actix_web::HttpResponse {
        let req = actix_web::test::TestRequest::with_uri(uri)
            .method(actix_web::http::Method::GET)
            .to_request();
        request(req).await
    }

    async fn get_with_peer_addr(
        uri: &str,
        addr: SocketAddr
    ) -> actix_web::HttpResponse {
        let req = actix_web::test::TestRequest::with_uri(uri)
            .method(actix_web::http::Method::GET)
            .peer_addr(addr)
            .to_request();
        request(req).await
    }

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

        let res = get("/api/channels/GR/xx/stream").await;
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

        for decode in &decode_values {
            let res = get(format!(
                "/api/channels/WOWOW/ch/services/2/stream?decode={}",
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

        for decode in &decode_values {
            let res = get(format!("/api/services/2/stream?decode={}",
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

        for decode in &decode_values {
            let res = get(format!("/api/programs/200001/stream?decode={}",
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

    #[actix_rt::test]
    async fn test_get_docs() {
        let res = get("/api/docs").await;
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[actix_rt::test]
    async fn test_access_control() {
        let res = get_with_peer_addr(
            "/api/version", "127.0.0.1:10000".parse().unwrap()).await;
        assert_eq!(res.status(), actix_web::http::StatusCode::OK);

        let res = get_with_peer_addr(
            "/api/version", "8.8.8.8:10000".parse().unwrap()).await;
        assert_eq!(res.status(), actix_web::http::StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_is_private_ip_addr() {
        assert!(is_private_ip_addr("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip_addr("192.168.0.1".parse().unwrap()));
        assert!(!is_private_ip_addr("8.8.8.8".parse().unwrap()));
    }

    #[actix_rt::test]
    async fn test_do_streaming() {
        let result = do_streaming(futures::stream::empty(), 1000).await;
        assert_matches!(result, Err(Error::ProgramNotFound));

        let result = do_streaming(futures::stream::pending(), 1).await;
        assert_matches!(result, Err(Error::StreamingTimedOut));
    }

    fn config_for_test() -> Arc<Config> {
        let mut config = Config::default();
        // Disable all filters
        config.filters.pre_filter = String::new();
        config.filters.service_filter = String::new();
        config.filters.program_filter = String::new();
        config.filters.post_filter = String::new();
        // Disable tracking airtime
        config.recorder.track_airtime_command = "true".to_string();
        // "/dev/null" is enough to test
        config.mirakurun.openapi_json = "/dev/null".to_string();

        Arc::new(config)
    }

    fn tuner_manager_for_test() -> Addr<TunerManagerActor> {
        TunerManagerActor::mock(Box::new(|msg, ctx| {
            if let Some(_) = msg.downcast_ref::<QueryTunersMessage>() {
                Box::<Option<Result<Vec<MirakurunTuner>, Error>>>::new(
                    Some(Ok(Vec::new())))
            } else if let Some(msg) = msg.downcast_ref::<StartStreamingMessage>() {
                if msg.channel.channel == "ch" {
                    let (mut tx, stream) = BroadcasterStream::new_for_test();
                    let _ = tx.try_send(Bytes::from("hi"));
                    let result = Ok(MpegTsStream::new(
                        Default::default(), stream, ctx.address().recipient()));
                    Box::<Option<Result<MpegTsStream, Error>>>::new(Some(result))
                } else {
                    let (_, stream) = BroadcasterStream::new_for_test();
                    let result = Ok(MpegTsStream::new(
                        Default::default(), stream, ctx.address().recipient()));
                    Box::<Option<Result<MpegTsStream, Error>>>::new(Some(result))
                }
            } else if let Some(_) = msg.downcast_ref::<StopStreamingMessage>() {
                Box::<Option<()>>::new(Some(()))
            } else {
                unimplemented!();
            }
        })).start()
    }

    fn epg_for_test() -> Addr<EpgActor> {
        EpgActor::mock(Box::new(|msg, _| {
            if let Some(_) = msg.downcast_ref::<QueryChannelsMessage>() {
                Box::<Option<Result<Vec<MirakurunChannel>, Error>>>::new(
                    Some(Ok(Vec::new())))
            } else if let Some(msg) = msg.downcast_ref::<QueryChannelMessage>() {
                let result = if msg.channel == "0" {
                    Err(Error::ChannelNotFound)
                } else {
                    Ok(EpgChannel {
                        name: "test".to_string(),
                        channel_type: msg.channel_type,
                        channel: msg.channel.clone(),
                        extra_args: "".to_string(),
                        services: Vec::new(),
                        excluded_services: Vec::new(),
                    })
                };
                Box::<Option<Result<EpgChannel, Error>>>::new(Some(result))
            } else if let Some(_) = msg.downcast_ref::<QueryServicesMessage>() {
                Box::<Option<Result<Vec<EpgService>, Error>>>::new(
                    Some(Ok(Vec::new())))
            } else if let Some(msg) = msg.downcast_ref::<QueryServiceMessage>() {
                let result = match msg {
                    QueryServiceMessage::ByNidSid { nid, sid } => {
                        if sid.value() == 0 {
                            Err(Error::ServiceNotFound)
                        } else {
                            let channel = if sid.value() == 1 {
                                "ch"
                            } else {
                                ""
                            };
                            Ok(EpgService {
                                nid: *nid,
                                tsid: 0.into(),
                                sid: *sid,
                                service_type: 1,
                                logo_id: 0,
                                remote_control_key_id: 0,
                                name: "test".to_string(),
                                channel: EpgChannel {
                                    name: "test".to_string(),
                                    channel_type: ChannelType::GR,
                                    channel: channel.to_string(),
                                    extra_args: "".to_string(),
                                    services: Vec::new(),
                                    excluded_services: Vec::new(),
                                },
                            })
                        }
                    }
                };
                Box::<Option<Result<EpgService, Error>>>::new(Some(result))
            } else if let Some(msg) = msg.downcast_ref::<QueryClockMessage>() {
                let result = match msg.triple.sid().value() {
                    0 => Err(Error::ClockNotSynced),
                    _ => Ok(Clock { pcr: 0, time: 0 }),
                };
                Box::<Option<Result<Clock, Error>>>::new(Some(result))
            } else if let Some(_) = msg.downcast_ref::<QueryProgramsMessage>() {
                Box::<Option<Result<Vec<EpgProgram>, Error>>>::new(
                    Some(Ok(Vec::new())))
            } else if let Some(msg) = msg.downcast_ref::<QueryProgramMessage>() {
                let result = match msg {
                    QueryProgramMessage::ByNidSidEid { nid, sid, eid } => {
                        if eid.value() == 0 {
                            Err(Error::ProgramNotFound)
                        } else {
                            Ok(EpgProgram::new(
                                (*nid, 0.into(), *sid, *eid).into()))
                        }
                    }
                };
                Box::<Option<Result<EpgProgram, Error>>>::new(Some(result))
            } else if let Some(_) = msg.downcast_ref::<RemoveAirtimeMessage>() {
                Box::<Option<()>>::new(Some(()))
            } else {
                unimplemented!();
            }
        })).start()
    }
}
