pub(in super) mod services;
pub(in super) mod stream;

use super::*;

pub(super) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunChannel>>, Error>
where
    E: Call<epg::QueryChannels>,
{
    state
        .epg
        .call(epg::QueryChannels)
        .await
        .map(Json::from)
        .map_err(Error::from)
}
