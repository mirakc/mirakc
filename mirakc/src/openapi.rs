use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;
use clap::ValueEnum;

use mirakc_core::*;

/// Output the OpenAPI document.
///
/// The same OpenAPI document can be obtained from the /api/docs endpoint of a mirakc server using
/// the same config.yml (or config.toml).
///
/// This command provides the same functionality without launching the mirakc server.  This is
/// useful if you want to generate a mirakc client from the OpenAPI document by using a tool such
/// as 'openapi-generator-cli':
///
///   mirakc openapi openapi.json
///   openapi-generator-cli generate -i openapi.json -g rust \
///     -o mirakc-client --package-name mirakc-client
///
#[derive(Args)]
#[clap(verbatim_doc_comment)]
pub struct CommandLine {
    /// Output format.
    #[arg(short, long, value_enum, default_value = "json")]
    format: Format,

    /// Output file.
    ///
    /// The contents will be output to STDOUT if the output file is not specified.
    #[arg()]
    file: Option<PathBuf>,
}

impl CommandLine {
    pub fn has_file(&self) -> bool {
        self.file.is_some()
    }
}

#[derive(Copy, Clone, ValueEnum)]
enum Format {
    Json,
    Yaml,
}

pub async fn main(config: Arc<config::Config>, cl: &CommandLine) {
    let docs = web::api::Docs::generate(&config);

    let contents = match cl.format {
        Format::Json => docs.to_json().unwrap(),
        Format::Yaml => docs.to_yaml().unwrap(),
    };

    match cl.file {
        Some(ref file) => tokio::fs::write(file, contents).await.unwrap(),
        None => println!("{contents}"),
    }
}
