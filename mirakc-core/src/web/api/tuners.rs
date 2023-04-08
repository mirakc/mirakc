use super::*;

/// Lists tuners enabled in `config.yml`.
#[utoipa::path(
    get,
    path = "/tuners",
    responses(
        (status = 200, description = "OK", body = [MirakurunTuner]),
        (status = 500, description = "Internal Server Error"),
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

/// Gets a tuner model.
#[utoipa::path(
    get,
    path = "/tuners/{index}",
    params(
        ("index" = usize, Path, description = "Tuner index"),
    ),
    responses(
        (status = 200, description = "OK", body = MirakurunTuner),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error"),
    ),
    // Specifying a correct operation ID is needed for working with
    // mirakurun.Client properly.
    operation_id = "getTuner",
)]
pub(super) async fn get<T>(
    State(TunerManagerExtractor(tuner_manager)): State<TunerManagerExtractor<T>>,
    Path(index): Path<usize>,
) -> Result<Json<MirakurunTuner>, Error>
where
    T: Call<tuner::QueryTuner>,
{
    let tuner = tuner_manager.call(tuner::QueryTuner(index)).await??;
    Ok(Json(tuner))
}
