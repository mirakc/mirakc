use super::*;

use std::fmt::Write as _;

use axum::extract::Host;
use axum::extract::Query;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use chrono_jst::Jst;

use crate::string_table::StringTable;

use crate::web::api::stream::determine_stream_content_type;
use crate::web::escape::escape;

/// Get a M3U8 playlist containing all available services.
#[utoipa::path(
    get,
    path = "/iptv/playlist",
    responses(
        (status = 200, description = "OK", content_type = "application/x-mpegURL", body = String),
    ),
)]
pub(super) async fn playlist<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Host(host): Host,
    Qs(filter_setting): Qs<FilterSetting>,
) -> impl IntoResponse
where
    E: Call<epg::QueryServices>,
{
    do_playlist(&state.config, &state.epg, &host, filter_setting).await
}

async fn do_playlist<E>(
    config: &Config,
    epg: &E,
    host: &str,
    mut filter_setting: FilterSetting,
) -> Result<Response<String>, Error>
where
    E: Call<epg::QueryServices>,
{
    const INITIAL_BUFSIZE: usize = 8 * 1024; // 8KB

    filter_setting.decode = true; // always decode
    let query = serde_qs::to_string(&filter_setting).expect("Never fails");

    let services = epg.call(epg::QueryServices).await?;

    // TODO: URL scheme

    let mut buf = String::with_capacity(INITIAL_BUFSIZE);
    write!(buf, "#EXTM3U\n")?;
    for sv in services.values() {
        let id = MirakurunServiceId::from(sv.triple());
        let logo_url = format!("http://{}/api/services/{}/logo", host, id.value());
        // The following format is compatible with EPGStation.
        // See API docs for the `/api/channel.m3u8` endpoint.
        //
        // U+3000 (IDEOGRAPHIC SPACE) at the end of each line is required for
        // avoiding garbled characters in `ＮＨＫＢＳプレミアム`.  Kodi or PVR
        // IPTV Simple Client seems to treat it as Latin-1 when removing U+3000.
        match sv.service_type {
            0x01 | 0xA1 | 0xA5 | 0xAD => {
                // video
                // Special optimization for IPTV Simple Client.
                //
                // Explicitly specifying the mime type of each channel avoids
                // redundant requests.
                match determine_stream_content_type(&config, &filter_setting) {
                    "video/MP2T" => {
                        // The mime type MUST be `video/mp2t`.
                        // See StreamUtils::GetStreamType() in
                        // src/iptvsimple/utilities/StreamUtils.cpp in
                        // kodi-pvr/pvr.iptvsimple.
                        write!(buf, "#KODIPROP:mimetype=video/mp2t\n")?;
                    }
                    mimetype => {
                        write!(buf, "#KODIPROP:mimetype={}\n", mimetype)?;
                    }
                }
                write!(buf, r#"#EXTINF:-1 tvg-id="{}""#, id.value())?;
                if config.resource.logos.contains_key(&sv.triple()) {
                    write!(buf, r#" tvg-logo="{}""#, logo_url)?;
                }
                write!(
                    buf,
                    r#" group-title="{}", {}　"#,
                    sv.channel.channel_type, sv.name
                )?;
            }
            0x02 | 0xA2 | 0xA6 => {
                // audio
                write!(buf, r#"#EXTINF:-1 tvg-id="{}""#, id.value())?;
                if config.resource.logos.contains_key(&sv.triple()) {
                    write!(buf, r#" tvg-logo="{}""#, logo_url)?;
                }
                write!(
                    buf,
                    r#" group-title="{}-Radio" radio=true, {}　"#,
                    sv.channel.channel_type, sv.name
                )?;
            }
            _ => unreachable!(),
        }
        write!(
            buf,
            "\nhttp://{}/api/services/{}/stream?{}\n",
            host,
            id.value(),
            query
        )?;
    }

    Ok(Response::builder()
        .header(CONTENT_TYPE, "application/x-mpegurl; charset=UTF-8")
        .body(buf)?)
}

/// Gets an XMLTV document containing all TV program information.
#[utoipa::path(
    get,
    path = "/iptv/epg",
    responses(
        (status = 200, description = "OK", content_type = "application/xml", body = String),
    ),
)]
pub(super) async fn epg<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Host(host): Host,
    Query(query): Query<IptvEpgQuery>,
) -> impl IntoResponse
where
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryServices>,
{
    do_epg(&state.config, &state.string_table, &state.epg, &host, query).await
}

/// Gets an XMLTV document containing all TV program information.
///
/// For compatibility with Mirakurun.
#[utoipa::path(
    get,
    path = "/iptv/xmltv",
    responses(
        (status = 200, description = "OK", content_type = "application/xml", body = String),
    ),
)]
pub(super) async fn xmltv<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Host(host): Host,
) -> impl IntoResponse
where
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryServices>,
{
    // Mirakurun doesn't support the days query parameter and returns all
    // programs.
    let query = IptvEpgQuery { days: 10 };
    do_epg(&state.config, &state.string_table, &state.epg, &host, query).await
}

async fn do_epg<E>(
    config: &Config,
    string_table: &StringTable,
    epg: &E,
    host: &str,
    query: IptvEpgQuery,
) -> Result<Response<String>, Error>
where
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryServices>,
{
    const INITIAL_BUFSIZE: usize = 8 * 1024 * 1024; // 8MB
    const DATETIME_FORMAT: &'static str = "%Y%m%d%H%M%S %z";

    let end_after = Jst::midnight();
    let start_before = end_after + chrono::Duration::days(query.days as i64);

    let services = epg.call(epg::QueryServices).await?;

    // TODO: URL scheme

    let mut buf = String::with_capacity(INITIAL_BUFSIZE);
    write!(buf, r#"<?xml version="1.0" encoding="UTF-8" ?>"#)?;
    write!(buf, r#"<!DOCTYPE tv SYSTEM "xmltv.dtd">"#)?;
    write!(
        buf,
        r#"<tv generator-info-name="{}">"#,
        escape(&server_name())
    )?;
    for sv in services.values() {
        let id = MirakurunServiceId::from(sv.triple());
        let logo_url = format!("http://{}/api/services/{}/logo", host, id.value());
        write!(buf, r#"<channel id="{}">"#, id.value())?;
        write!(
            buf,
            r#"<display-name lang="ja">{}</display-name>"#,
            escape(&sv.name)
        )?;
        if config.resource.logos.contains_key(&sv.triple()) {
            write!(buf, r#"<icon src="{}" />"#, logo_url)?;
        }
        write!(buf, r#"</channel>"#)?;
    }
    for triple in services.keys() {
        let programs = epg
            .call(epg::QueryPrograms {
                service_triple: triple.clone(),
            })
            .await?;
        for pg in programs
            .values()
            .filter(|pg| pg.name.is_some())
            .filter(|pg| pg.start_at.unwrap() < start_before && pg.end_at().unwrap() > end_after)
        {
            let id = MirakurunServiceId::from(pg.quad);
            write!(
                buf,
                r#"<programme start="{}" stop="{}" channel="{}">"#,
                pg.start_at.unwrap().format(DATETIME_FORMAT),
                pg.end_at().unwrap().format(DATETIME_FORMAT),
                id.value()
            )?;
            if let Some(name) = pg.name.as_ref() {
                write!(buf, r#"<title lang="ja">{}</title>"#, escape(&name))?;
            }
            if let Some(desc) = pg.description.as_ref() {
                write!(buf, r#"<desc lang="ja">"#)?;
                write!(buf, "{}", escape(&desc))?;
                if let Some(extended) = pg.extended.as_ref() {
                    for (key, value) in extended.iter() {
                        if key.is_empty() {
                            write!(buf, "{}", escape(&value))?;
                        } else {
                            write!(buf, "\n{}\n{}", escape(&key), escape(&value))?;
                        }
                    }
                }
                write!(buf, r#"</desc>"#)?;
            }
            if let Some(genres) = pg.genres.as_ref() {
                for genre in genres.iter() {
                    let genre_str = &string_table.genres[genre.lv1 as usize].genre;
                    let subgenre_str =
                        &string_table.genres[genre.lv1 as usize].subgenres[genre.lv2 as usize];
                    if subgenre_str.is_empty() {
                        write!(
                            buf,
                            r#"<category lang="ja">{}</category>"#,
                            escape(&genre_str)
                        )?;
                    } else {
                        write!(
                            buf,
                            r#"<category lang="ja">{} / {}</category>"#,
                            escape(&genre_str),
                            escape(&subgenre_str)
                        )?;
                    }
                }
            }
            write!(buf, r#"</programme>"#)?;
        }
    }
    write!(buf, r#"</tv>"#)?;

    Ok(Response::builder()
        .header(CONTENT_TYPE, "application/xml; charset=UTF-8")
        .body(buf)?)
}
