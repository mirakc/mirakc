pub(super) mod services;
pub(super) mod stream;

use super::*;

/// Lists channels.
#[utoipa::path(
    get,
    path = "/channels",
    responses(
        (status = 200, description = "OK", body = [MirakurunChannel]),
        (status = 505, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getChannels",
)]
pub(super) async fn list<E>(
    State(EpgExtractor(epg)): State<EpgExtractor<E>>,
) -> Result<Json<Vec<MirakurunChannel>>, Error>
where
    E: Call<epg::QueryChannels>,
{
    epg.call(epg::QueryChannels)
        .await
        .map(Json::from)
        .map_err(Error::from)
}
