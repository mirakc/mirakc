use super::*;

use crate::onair;

/// List on-air programs.
#[utoipa::path(
    get,
    path = "/onair",
    responses(
        (status = 200, description = "OK", body = [WebOnairProgram]),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(super) async fn list<O>(
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
) -> Result<Json<Vec<WebOnairProgram>>, Error>
where
    O: Call<onair::QueryOnairPrograms>,
{
    onair_manager
        .call(onair::QueryOnairPrograms)
        .await
        .map(|onair_programs| {
            onair_programs
                .into_iter()
                .map(WebOnairProgram::from)
                .collect_vec()
        })
        .map(Json::from)
        .map_err(Error::from)
}

/// Gets an on-air program of a specified service.
#[utoipa::path(
    get,
    path = "/onair/{service_id}",
    params(
        ("service_id" = u64, Path, description = "Mirakurun service ID"),
    ),
    responses(
        (status = 200, description = "OK", body = [WebOnairProgram]),
        (status = 404, description = "Not Found"),
        (status = 505, description = "Internal Server Error"),
    ),
)]
pub(super) async fn get<O>(
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
    Path(service_id): Path<ServiceId>,
) -> Result<Json<WebOnairProgram>, Error>
where
    O: Call<onair::QueryOnairProgram>,
{
    onair_manager
        .call(onair::QueryOnairProgram { service_id })
        .await?
        .map(|onair_program| (service_id, onair_program))
        .map(WebOnairProgram::from)
        .map(Json::from)
        .map_err(Error::from)
}
