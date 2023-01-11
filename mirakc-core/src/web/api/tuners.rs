use super::*;

/// Lists tuners enabled in `config.yml`.
#[utoipa::path(
    get,
    path = "/tuners",
    responses(
        (status = 200, description = "OK", body = [MirakurunTuner]),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getTuners",
)]
pub(super) async fn list<T>(
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
) -> Result<Json<Vec<MirakurunTuner>>, Error>
where
    T: Call<tuner::QueryTuners>,
{
    tuner_manager
        .call(tuner::QueryTuners)
        .await
        .map(Json::from)
        .map_err(Error::from)
}
