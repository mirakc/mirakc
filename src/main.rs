mod airtime_tracker;
mod broadcaster;
mod chunk_stream;
mod clock_synchronizer;
mod command_util;
mod config;
mod datetime_ext;
mod eit_feeder;
mod epg;
mod error;
//mod fs_util;
mod job;
mod models;
mod mpeg_ts_stream;
mod service_scanner;
mod tokio_snippet;
mod tuner;
mod web;

use clap;
use tracing_subscriber;

use crate::error::Error;

#[actix_rt::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::filter::EnvFilter::from_default_env())
        .init();

    let args = clap::App::new(clap::crate_name!())
        .version(clap::crate_version!())
        .about(clap::crate_description!())
        .arg(clap::Arg::with_name("config")
             .short("c")
             .long("config")
             .takes_value(true)
             .value_name("FILE")
             .env("MIRAKC_CONFIG")
             .help("Path to a configuration file in a YAML format")
             .long_help(
                 "Path to a configuration file in a YAML format.\n\
                  \n\
                  The MIRAKC_CONFIG environment variable is used if this \
                  option is not specified.  Its value has to be an absolute \
                  path.\n\
                  \n\
                  See README.md for details of the YAML format."))
        .get_matches();

    let config_path = args.value_of("config").expect(
        "--config option or MIRAKC_CONFIG environment must be specified");

    let config = config::load(config_path);

    let tuner_manager = tuner::start(config.clone());

    let epg = epg::start(config.clone());

    let eit_feeder = eit_feeder::start(
        config.clone(), tuner_manager.clone(), epg.clone());

    let _job_manager = job::start(
        config.clone(), tuner_manager.clone(), epg.clone(), eit_feeder.clone());

    web::serve(config.clone(), tuner_manager.clone(), epg.clone()).await?;

    Ok(())
}
