use std::path::PathBuf;

use structopt::StructOpt;

use mirakc_core::error::Error;
use mirakc_core::tracing_ext::init_tracing;
use mirakc_core::*;

#[derive(StructOpt)]
#[structopt(about)]
struct Opt {
    /// Path to a configuration file in a YAML format.
    ///
    /// The MIRAKC_CONFIG environment variable is used if this option is not
    /// specified.  Its value has to be an absolute path.
    ///
    /// See docs/config.md for details of the YAML format.
    #[structopt(short, long, env = "MIRAKC_CONFIG")]
    config: PathBuf,

    /// Logging format.
    #[structopt(
        long,
        env = "MIRAKC_LOG_FORMAT",
        possible_values = &["text", "json"],
        default_value = "text",
    )]
    log_format: String,
}

#[actix::main]
async fn main() -> Result<(), Error> {
    let opt = Opt::from_args();

    init_tracing(&opt.log_format);

    let config = config::load(&opt.config);
    let string_table = string_table::load(&config.resource.strings_yaml);

    let tuner_manager = tuner::start(config.clone());

    let timeshift_manager = timeshift::start(config.clone(), tuner_manager.clone());

    let epg = epg::start(config.clone(), vec![timeshift_manager.clone().recipient()]);

    let eit_feeder = eit_feeder::start(config.clone(), tuner_manager.clone(), epg.clone());

    let _job_manager = job::start(
        config.clone(),
        tuner_manager.clone(),
        epg.clone(),
        eit_feeder.clone(),
    );

    web::serve(
        config.clone(),
        string_table.clone(),
        tuner_manager.clone(),
        epg.clone(),
        timeshift_manager.clone(),
    )
    .await?;

    Ok(())
}
