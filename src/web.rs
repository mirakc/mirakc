use actix_web;
use cfg_if;
use chrono::{Duration, Utc};
use futures::future::Future;
use serde::Deserialize;

use crate::config::Config;
use crate::error::Error;
use crate::messages::*;
use crate::models::*;
use crate::tuner::TunerUser;

pub fn start(config: &Config) -> Result<(), Error> {
    actix_web::HttpServer::new(
        move || {
            actix_web::App::new()
                .wrap(actix_web::middleware::Logger::default())
                .wrap(actix_web::middleware::DefaultHeaders::new()
                      .header("Server", server_name()))
                .service(create_api_service())
        })
        .bind((config.server.address.as_str(), config.server.port))?
        .workers(config.server.workers)
        .start();
    Ok(())
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        use tests::resource_manager_mock as resource_manager;
    } else {
        use crate::resource_manager;
    }
}

fn server_name() -> String {
    format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

// rest api

type ApiResult = Box<dyn Future<Item = actix_web::HttpResponse,
                                Error = actix_web::Error>>;

impl actix_web::ResponseError for Error {}

fn create_api_service() -> impl actix_web::dev::HttpServiceFactory {
    actix_web::web::scope("/api")
        .service(get_version)
        .service(get_status)
        .service(get_channels)
        .service(get_services)
        .service(get_programs)
        .service(get_tuners)
        .service(get_channel_stream)
        .service(get_service_stream)
        .service(get_program_stream)
}

#[actix_web::get("/version")]
fn get_version() -> impl actix_web::Responder {
    actix_web::HttpResponse::Ok().json(env!("CARGO_PKG_VERSION"))
}

#[actix_web::get("/status")]
fn get_status() -> impl actix_web::Responder {
    actix_web::HttpResponse::Ok().content_type("application/json").body("{}")
}

#[actix_web::get("/channels")]
fn get_channels() -> ApiResult {
    let msg = QueryChannelsMessage;
    Box::new(
        resource_manager::query_channels(msg)
            .map(|channels| actix_web::HttpResponse::Ok().json(channels))
            .from_err()
    )
}

#[actix_web::get("/services")]
fn get_services() -> ApiResult {
    let msg = QueryServicesMessage;
    Box::new(
        resource_manager::query_services(msg)
            .map(|services| actix_web::HttpResponse::Ok().json(services))
            .from_err()
    )
}

#[actix_web::get("/programs")]
fn get_programs() -> ApiResult {
    let msg = QueryProgramsMessage;
    Box::new(
        resource_manager::query_programs(msg)
            .map(|programs| {
                // A temporal vector of &ProgramModel is created for
                // serialization.
                //
                // Serde doesn't support serializing an iterator natively.  That
                // means an additional implementation is needed for the
                // serialization.
                //
                // See https://stackoverflow.com/questions/34399461.
                let values: Vec<&ProgramModel> = programs.values().collect();
                actix_web::HttpResponse::Ok().json(values)
            })
            .from_err()
    )
}

#[actix_web::get("/tuners")]
fn get_tuners() -> ApiResult {
    let msg = QueryTunersMessage;
    Box::new(
        resource_manager::query_tuners(msg)
            .map(|tuners| actix_web::HttpResponse::Ok().json(tuners))
            .from_err()
    )
}

#[actix_web::get("/channels/{channel_type}/{channel}/stream")]
fn get_channel_stream(
    path: actix_web::web::Path<ChannelPath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    get_stream(path.into_inner(), query.into_inner(), user)
}

#[actix_web::get("/services/{sid}/stream")]
fn get_service_stream(
    path: actix_web::web::Path<ServicePath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    get_stream(path.into_inner(), query.into_inner(), user)
}

#[actix_web::get("/programs/{eid}/stream")]
fn get_program_stream(
    path: actix_web::web::Path<ProgramPath>,
    query: actix_web::web::Query<StreamQuery>,
    user: TunerUser
) -> ApiResult {
    get_stream(path.into_inner(), query.into_inner(), user)
}

// helpers

fn get_stream<P>(
    path: P,
    query: StreamQuery,
    user: TunerUser
) -> ApiResult
where
    P: Into<OpenTunerBy>
{
    let duration = query.duration();
    let preprocess = query.preprocess();
    let postprocess = query.postprocess();
    let msg = OpenTunerMessage {
        by: path.into(), user, duration, preprocess, postprocess,
    };
    Box::new(
        resource_manager::open_tuner(msg)
            .map(|output| {
                actix_web::HttpResponse::Ok()
                    .set_header("content-type", "video/MP2T")
                    .streaming(output.into_stream())
            })
            .from_err()
    )
}

// extractors

#[derive(Deserialize)]
struct ChannelPath {
    channel_type: ChannelType,
    channel: String,
}

impl Into<OpenTunerBy> for ChannelPath {
    fn into(self) -> OpenTunerBy {
        OpenTunerBy::Channel {
            channel_type: self.channel_type,
            channel: self.channel,
        }
    }
}

#[derive(Deserialize)]
struct ServicePath(u64);

impl Into<OpenTunerBy> for ServicePath {
    fn into(self) -> OpenTunerBy {
        OpenTunerBy::Service { id: self.0 }
    }
}

#[derive(Deserialize)]
struct ProgramPath(u64);

impl Into<OpenTunerBy> for ProgramPath {
    fn into(self) -> OpenTunerBy {
        OpenTunerBy::Program { id: self.0 }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct StreamQuery {
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    preprocess: Option<bool>,
    #[serde(default)]
    postprocess: Option<bool>,

    // For compatibility with Mirakurun.
    // The post-filter parameter can override the decode parameter.
    #[serde(default)]
    decode: u8,  // default: 0
}

impl StreamQuery {
    fn duration(&self) -> Option<Duration> {
        match self.duration {
            Some(ref s) => s.parse::<i64>().ok().map(Duration::seconds),
            None => None,
        }
    }

    fn preprocess(&self) -> bool {
        match self.preprocess {
            Some(preprocess) => preprocess,
            None => false,
        }
    }

    fn postprocess(&self) -> bool {
        match (self.postprocess, self.decode) {
            (Some(postprocess), _) => postprocess,
            (None, decode) => decode != 0,  // for compatibility with Mirakurun
        }
    }
}

impl actix_web::FromRequest for TunerUser {
    type Error = actix_web::Error;
    type Future = Result<Self, Self::Error>;
    type Config = ();

    fn from_request(
        req: &actix_web::HttpRequest,
        _: &mut actix_web::dev::Payload
    ) -> Self::Future {
        // Compatible with JavaScript `Date.now()` used in Mirakurun.
        let timestamp = Utc::now().timestamp_millis();

        let id = match req.connection_info().remote() {
            Some(ref ip) => format!("{}#{}", ip, timestamp),
            None => format!("{}", timestamp),
        };

        let ua = req.headers().get_all(actix_web::http::header::USER_AGENT)
            .last()
            .map(|value| {
                value.to_str().ok().map_or(String::new(), |s| s.to_string())
            });

        let prio = req.headers().get_all("x-mirakurun-priority")
            .filter_map(|value| value.to_str().ok())
            .filter_map(|value| value.parse().ok())
            .max()
            .unwrap_or(0);

        Ok(TunerUser::new(id, ua, prio))
    }
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
    pub mod resource_manager_mock {
        use super::*;
        use std::collections::HashMap;
        use std::sync::Arc;
        use crate::tuner::TunerOutput;

        pub fn query_channels(
            _: QueryChannelsMessage
        ) -> impl Future<Item = Vec<ChannelModel>, Error = Error> {
            futures::future::ok(Vec::new())
        }

        pub fn query_services(
            _: QueryServicesMessage
        ) -> impl Future<Item = Arc<Vec<ServiceModel>>, Error = Error> {
            futures::future::ok(Arc::new(Vec::new()))
        }

        pub fn query_programs(
            _: QueryProgramsMessage
        ) -> impl Future<Item = Arc<HashMap<u64, ProgramModel>>,
                         Error = Error> {
            futures::future::ok(Arc::new(HashMap::new()))
        }

        pub fn query_tuners(
            _: QueryTunersMessage
        ) -> impl Future<Item = Vec<TunerModel>, Error = Error> {
            futures::future::ok(Vec::new())
        }

        pub fn open_tuner(
            _: OpenTunerMessage
        ) -> impl Future<Item = TunerOutput, Error = Error> {
            futures::future::ok(TunerOutput::new(0, 0, None))
        }
    }

    fn request(
        method: actix_web::http::Method,
        uri: &str
    ) -> actix_web::HttpResponse {
        let mut app = actix_web::test::init_service(
            actix_web::App::new().service(create_api_service()));
        let req = actix_web::test::TestRequest::with_uri(uri)
            .method(method).to_request();
        actix_web::test::call_service(&mut app, req).into()
    }

    macro_rules! impl_method {
        ($method:ident, $METHOD:ident) => {
            fn $method(uri: &str) -> actix_web::HttpResponse {
                request(actix_web::http::Method::$METHOD, uri)
            }
        }
    }

    impl_method!(get, GET);

    #[test]
    fn test_get_unknown() {
        let res = get("/api/unknown");
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_get_version() {
        let res = get("/api/version");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_get_status() {
        let res = get("/api/status");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_get_channels() {
        let res = get("/api/channels");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_get_tuners() {
        let res = get("/api/tuners");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_get_channel_stream() {
        let res = get("/api/channels/GR/ch/stream");
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/channels/WOWOW/ch/stream");
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        let valid_pairs =
            ["GR", "BS", "CS", "SKY"].iter().zip(&decode_values);
        for (channel_type, decode) in valid_pairs {
            let res = get(format!("/api/channels/{}/ch/stream?decode={}",
                                  channel_type, decode).as_str());
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!("/api/channels/WOWOW/ch/stream?decode={}",
                                  decode).as_str());
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }

        let res = get("/api/channels/GR/ch/stream?duration=1");
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/channels/GR/ch/stream?duration=-");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_get_service_stream() {
        let res = get("/api/services/1/stream");
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/services/x/stream");
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        for decode in &decode_values {
            let res = get(format!("/api/services/1/stream?decode={}",
                                  decode).as_str());
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!("/api/services/x/stream?decode={}",
                                  decode).as_str());
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }

        let res = get("/api/services/1/stream?duration=1");
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/services/1/stream?duration=-");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_get_program_stream() {
        let res = get("/api/programs/1/stream");
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/programs/x/stream");
        assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);

        let decode_values = [0, 1];

        for decode in &decode_values {
            let res = get(format!("/api/programs/1/stream?decode={}",
                                  decode).as_str());
            assert!(res.status() == actix_web::http::StatusCode::OK);
        }

        for decode in &decode_values {
            let res = get(format!("/api/programs/x/stream?decode={}",
                                  decode).as_str());
            assert!(res.status() == actix_web::http::StatusCode::NOT_FOUND);
        }

        let res = get("/api/programs/1/stream?duration=1");
        assert!(res.status() == actix_web::http::StatusCode::OK);

        let res = get("/api/programs/1/stream?duration=-");
        assert!(res.status() == actix_web::http::StatusCode::OK);
    }

    #[test]
    fn test_stream_query() {
        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "").unwrap().into_inner();
        assert_eq!(query.duration(), None);
        assert_eq!(query.preprocess(), false);
        assert_eq!(query.postprocess(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "duration=1").unwrap().into_inner();
        assert_eq!(query.duration(), Some(Duration::seconds(1)));

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "duration=-").unwrap().into_inner();
        assert_eq!(query.duration(), None);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "duration").unwrap().into_inner();
        assert_eq!(query.duration(), None);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "duration=").unwrap().into_inner();
        assert_eq!(query.duration(), None);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "preprocess=true").unwrap().into_inner();
        assert_eq!(query.preprocess(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "preprocess=false").unwrap().into_inner();
        assert_eq!(query.preprocess(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "preprocess");
        assert!(query.is_err());

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "preprocess=");
        assert!(query.is_err());

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "preprocess=1");
        assert!(query.is_err());

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "postprocess=true").unwrap().into_inner();
        assert_eq!(query.postprocess(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "postrocess=false").unwrap().into_inner();
        assert_eq!(query.postprocess(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "decode=1").unwrap().into_inner();
        assert_eq!(query.postprocess(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "decode=0").unwrap().into_inner();
        assert_eq!(query.postprocess(), false);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "decode=2").unwrap().into_inner();
        assert_eq!(query.postprocess(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "postprocess=true&decode=0").unwrap().into_inner();
        assert_eq!(query.postprocess(), true);

        let query = actix_web::web::Query::<StreamQuery>::from_query(
            "postprocess=false&decode=1").unwrap().into_inner();
        assert_eq!(query.postprocess(), false);
    }
}
