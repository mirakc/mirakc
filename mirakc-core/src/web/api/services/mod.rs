pub(super) mod stream;

use super::*;

/// Lists services.
#[utoipa::path(
    get,
    path = "/services",
    responses(
        (status = 200, description = "OK", body = [MirakurunService]),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getServices",
)]
pub(super) async fn list<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
) -> Result<Json<Vec<MirakurunService>>, Error>
where
    E: Call<epg::QueryServices>,
{
    Ok(epg
        .call(epg::QueryServices)
        .await?
        .values()
        .cloned()
        .map(MirakurunService::from)
        .map(|mut service| {
            service.check_logo_existence(&config.resource);
            service
        })
        .collect::<Vec<MirakurunService>>()
        .into())
}

/// Gets a service.
#[utoipa::path(
    get,
    path = "/services/{id}",
    params(
        ("id" = u64, Path, description = "Mirakurun service ID"),
    ),
    responses(
        (status = 200, description = "OK", body = MirakurunService),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getService",
)]
pub(super) async fn get<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Json<MirakurunService>, Error>
where
    E: Call<epg::QueryService>,
{
    epg.call(epg::QueryService::ByMirakurunServiceId(id))
        .await?
        .map(MirakurunService::from)
        .map(|mut service| {
            service.check_logo_existence(&config.resource);
            Json(service)
        })
}

/// Gets a logo image of a service.
#[utoipa::path(
    get,
    path = "/services/{id}/logo",
    params(
        ("id" = u64, Path, description = "Mirakurun service ID"),
    ),
    responses(
        (status = 200, description = "OK", content_type = "image/png"),
        (status = 404, description = "Not Found"),
        (status = 503, description = "Logo Data Unavailable"),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getLogoImage",
)]
pub(super) async fn logo<E>(
    State(ConfigExtractor(config)): State<ConfigExtractor>,
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Response<StaticFileBody>, Error>
where
    E: Call<epg::QueryService>,
{
    let service = epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await??;

    match config.resource.logos.get(&service.id()) {
        Some(path) => {
            Ok(Response::builder()
                // TODO: The type should be specified in config.yml.
                .header(CONTENT_TYPE, "image/png")
                .body(StaticFileBody::new(path).await?)?)
        }
        None => Err(Error::NoLogoData),
    }
}

/// Lists TV programs of a service.
///
/// The list contains TV programs that have ended.
#[utoipa::path(
    get,
    path = "/services/{id}/programs",
    params(
        ("id" = u64, Path, description = "Mirakurun service ID"),
    ),
    responses(
        (status = 200, description = "OK", body = [MirakurunProgram]),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(super) async fn programs<E>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Json<Vec<MirakurunProgram>>, Error>
where
    E: Call<epg::QueryService>,
    E: Call<epg::QueryPrograms>,
{
    let service = epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await??;

    let programs = epg
        .call(epg::QueryPrograms {
            service_id: service.id(),
        })
        .await?
        .values()
        .cloned()
        .map(MirakurunProgram::from)
        .collect_vec();
    Ok(programs.into())
}
