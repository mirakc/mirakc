mod config;
mod datetime_ext;
mod epg;
mod error;
mod messages;
mod models;
mod resource_manager;
mod tuner;
mod web;

use std::env;

use actix::prelude::*;
use clap;
use pretty_env_logger;

use crate::config::Config;
use crate::error::Error;

fn main() -> Result<(), Error> {
    let args = clap::App::new(clap::crate_name!())
        .version(clap::crate_version!())
        .author(clap::crate_authors!("\n"))
        .about(clap::crate_description!())
        .arg(clap::Arg::with_name("config")
             .short("c")
             .long("config")
             .takes_value(true)
             .value_name("FILE")
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

    pretty_env_logger::init_timed();

    let config_yml = match args.value_of("config") {
        Some(config_yml) => config_yml.to_string(),
        None => env::var("MIRAKC_CONFIG")?,
    };
    let config = Config::load(&config_yml)?;
    let runner = System::new("mirakc");
    resource_manager::start(&config);
    web::start(&config)?;
    runner.run()?;
    Ok(())
}
