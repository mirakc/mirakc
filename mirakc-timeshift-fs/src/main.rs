mod filesystem;

use std::path::PathBuf;

use clap::Parser;
use clap::ValueEnum;

use crate::filesystem::TimeshiftFilesystem;
use mirakc_core::error::Error;

#[derive(Parser)]
#[command(author, version, about)]
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
    #[arg(long, value_enum, env = "MIRAKC_LOG_FORMAT", default_value = "text")]
    log_format: LogFormat,

    /// Additional mount options.
    ///
    /// The following mount options will be added internally:
    /// fsname=mirakc-timeshift,ro,noatime
    #[arg(short, long)]
    options: Vec<String>,

    /// Path to the mount point.
    #[arg()]
    mount_point: PathBuf,
}

#[derive(Clone, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

fn main() -> Result<(), Error> {
    let opt = Opt::parse();

    mirakc_core::tracing_ext::init_tracing(match opt.log_format {
        LogFormat::Text => "text",
        LogFormat::Json => "json",
    });

    let config = mirakc_core::config::load(&opt.config);

    let fs = TimeshiftFilesystem::new(config);

    let mut options = vec![
        fuser::MountOption::FSName("mirakc-timeshift".to_string()),
        fuser::MountOption::RO,
        fuser::MountOption::NoAtime,
    ];
    for option in opt.options.iter() {
        options.push(fuser::MountOption::CUSTOM(option.clone()));
    }

    // fuser::MountOption::AutoMount does NOT work properly when the process terminates by signals
    // like SIGTERM.  Because the process terminates before fuser::Session<FS>::drop() is called.
    //
    // For avoiding the situation above, we can use a signal handler in mirakc-timeshift-fs.
    // However, we adopt a simpler solution like below:
    //
    // ```shell
    // MOUNT_POINT="$1"
    //
    // unmount() {
    //   echo "Unmount $MOUNT_POINT"
    //   umount $MOUNT_POINT
    // }
    //
    // # Register a signal handler to unmount the mirakc-timeshift filesystem automatically
    // # before the process terminates.
    // trap 'unmount' EXIT
    //
    // # Then mount it.
    // mirakc-timeshift-fs $MOUNT_POINT
    // ```
    Ok(fuser::mount2(fs, &opt.mount_point, &options)?)
}
