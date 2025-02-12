mod v4;

use mirakc_core::*;

/// Migrate existing data to the new data format.
#[derive(clap::Args, Default)]
pub struct CommandLine {
    /// Always perform the migration.
    #[arg(short, long)]
    force: bool,
}

pub async fn main(config: &config::Config, cl: &CommandLine) {
    v4::migrate(config, cl);
}
