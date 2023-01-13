pub(in crate::web::api) mod stream;

use axum::headers::UserAgent;

use super::*;
use crate::onair;

/// Lists TV programs.
///
/// The list contains TV programs that have ended.
///
/// A newer Mirakurun returns information contained in EIT[schedule]
/// overridded by EIT[p/f] from this endpoint.  This may cause
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
    for &service_id in services.keys() {
        let programs = epg.call(epg::QueryPrograms { service_id }).await?;
        result.reserve(programs.len());
        result.extend(programs.values().cloned().map(MirakurunProgram::from));
    }
    Ok(result.into())
}

/// Gets a TV program.
///
/// ### A special hack for EPGStation
///
/// If the User-Agent header string starts with "EPGStation/", this endpoint
/// returns information contained in EIT[p/f] if it exists. Otherwise,
/// information contained in EIT[schedule] is returned.
///
/// EPGStation calls this endpoint in order to update the start time and the
/// duration of the TV program while recording.  The intention of this call is
/// assumed that EPGStation wants to get the TV program information equivalent
/// to EIT[p].  However, this endpoint should return information contained in
/// EIT[schedule] basically in a web API consistency point of view.  Information
/// contained in EIT[p/f] should be returned from other endpoints.
///
/// See also [/programs/{id}/stream](#/stream/getProgramStream).
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
pub(super) async fn get<E, O>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
    State(OnairProgramManagerExtractor(onair_manager)): State<OnairProgramManagerExtractor<O>>,
    user_agent: Option<TypedHeader<UserAgent>>,
    Path(id): Path<MirakurunProgramId>,
) -> Result<Json<MirakurunProgram>, Error>
where
    E: Call<epg::QueryProgram>,
    O: Call<onair::QueryOnairProgram>,
{
    let mut program = epg
        .call(epg::QueryProgram::ByMirakurunProgramId(id))
        .await??;

    if is_epgstation(&user_agent) {
        let msg = onair::QueryOnairProgram {
            service_id: program.id.into(),
        };
        if let Ok(Ok(onair_program)) = onair_manager.call(msg).await {
            if let Some(current) = onair_program.current {
                if current.id == program.id {
                    program = current.as_ref().clone();
                }
            }
            if let Some(next) = onair_program.next {
                if next.id == program.id {
                    program = next.as_ref().clone();
                }
            }
        }
    }

    Ok(Json(program.into()))
}

fn is_epgstation(user_agent: &Option<TypedHeader<UserAgent>>) -> bool {
    if let Some(TypedHeader(ref user_agent)) = user_agent {
        user_agent.as_str().starts_with("EPGStation/")
    } else {
        false
    }
}
