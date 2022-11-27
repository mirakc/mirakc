pub(in crate::web::api) mod stream;

use super::*;

pub(super) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunProgram>>, Error>
where
    E: Call<epg::QueryPrograms>,
    E: Call<epg::QueryServices>,
{
    let services = state.epg.call(epg::QueryServices).await?;
    let mut result = vec![];
    for triple in services.keys() {
        let programs = state
            .epg
            .call(epg::QueryPrograms {
                service_triple: triple.clone(),
            })
            .await?;
        result.reserve(programs.len());
        result.extend(programs.values().cloned().map(MirakurunProgram::from));
    }
    Ok(result.into())
}

pub(super) async fn get<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
    Path(id): Path<MirakurunProgramId>,
) -> Result<Json<MirakurunProgram>, Error>
where
    E: Call<epg::QueryProgram>,
{
    state
        .epg
        .call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await?
        .map(MirakurunProgram::from)
        .map(Json::from)
}
