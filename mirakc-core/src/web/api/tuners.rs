use super::*;

pub(super) async fn list<T, E, R, S>(
    State(state): State<Arc<AppState<T, E, R, S>>>,
) -> Result<Json<Vec<MirakurunTuner>>, Error>
where
    T: Call<tuner::QueryTuners>,
{
    state
        .tuner_manager
        .call(tuner::QueryTuners)
        .await
        .map(Json::from)
        .map_err(Error::from)
}
