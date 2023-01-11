pub(in crate::web::api) mod stream;

use super::*;

/// Lists TV programs.
///
/// The list contains TV programs that have ended.
#[utoipa::path(
    get,
    path = "/programs",
    responses(
        (status = 200, description = "OK", body = [MirakurunProgram]),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getPrograms",
)]
pub(super) async fn list<E>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
) -> Result<Json<Vec<MirakurunProgram>>, Error>
where
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryServices>,
{
    let services = epg.call(epg::QueryServices).await?;
    let mut result = vec![];
    for triple in services.keys() {
        let programs = epg
            .call(epg::QueryPrograms {
                service_triple: triple.clone(),
            })
            .await?;
        result.reserve(programs.len());
        result.extend(programs.values().cloned().map(MirakurunProgram::from));
    }
    Ok(result.into())
}

/// Gets a TV program.
#[utoipa::path(
    get,
    path = "/programs/{id}",
    params(
        ("id" = u64, Path, description = "Mirakurun program ID"),
    ),
    responses(
        (status = 200, description = "OK", body = [MirakurunProgram]),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getProgram",
)]
pub(super) async fn get<E>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    Path(id): Path<MirakurunProgramId>,
) -> Result<Json<MirakurunProgram>, Error>
where
    E: Call<epg::QueryProgram>,
{
    epg.call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await?
        .map(MirakurunProgram::from)
        .map(Json::from)
}
