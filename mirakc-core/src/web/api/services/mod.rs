pub(super) mod stream;

use super::*;

pub(super) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunService>>, Error>
where
    E: Call<epg::QueryServices>,
{
    Ok(state
        .epg
        .call(epg::QueryServices)
        .await?
        .values()
        .cloned()
        .map(MirakurunService::from)
        .map(|mut service| {
            service.check_logo_existence(&state.config.resource);
            service
        })
        .collect::<Vec<MirakurunService>>()
        .into())
}

pub(super) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Json<MirakurunService>, Error>
where
    E: Call<epg::QueryService>,
{
    state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await?
        .map(MirakurunService::from)
        .map(|mut service| {
            service.check_logo_existence(&state.config.resource);
            Json(service)
        })
}

pub(super) async fn logo<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Response<StaticFileBody>, Error>
where
    E: Call<epg::QueryService>,
{
    let service = state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await??;

    match state.config.resource.logos.get(&service.triple()) {
        Some(path) => {
            Ok(Response::builder()
                // TODO: The type should be specified in config.yml.
                .header(CONTENT_TYPE, "image/png")
                .body(StaticFileBody::new(path).await?)?)
        }
        None => Err(Error::NoLogoData),
    }
}

pub(super) async fn programs<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunServiceId>,
) -> Result<Json<Vec<MirakurunProgram>>, Error>
where
    E: Call<epg::QueryService>,
    E: Call<epg::QueryPrograms>,
{
    let service = state
        .epg
        .call(epg::QueryService::ByMirakurunServiceId(id))
        .await??;

    let programs = state
        .epg
        .call(epg::QueryPrograms {
            service_triple: service.triple(),
        })
        .await?
        .values()
        .cloned()
        .map(MirakurunProgram::from)
        .collect_vec();
    Ok(programs.into())
}
