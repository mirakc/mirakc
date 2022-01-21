mod filesystem;

use std::path::PathBuf;

use fuser;
use structopt::StructOpt;

use mirakc_core;
use mirakc_core::error::Error;
use crate::filesystem::TimeshiftFilesystem;

#[derive(StructOpt)]
#[structopt(about)]
struct Opt {
    /// Path to a configuration file in a YAML format.
    ///
    /// The MIRAKC_CONFIG environment variable is used if this option is not
    /// specified.  Its value has to be an absolute path.
    ///
    /// See docs/config.md for details of the YAML format.
    #[structopt(
        short,
        long,
        env = "MIRAKC_CONFIG",
    )]
    config: PathBuf,

    /// Logging format.
    #[structopt(
        long,
        env = "MIRAKC_LOG_FORMAT",
        possible_values = &["text", "json"],
        default_value = "text",
    )]
    log_format: String,

    /// Path to the mount point.
    #[structopt()]
    mount_point: PathBuf,
}

fn main() -> Result<(), Error> {
    let opt = Opt::from_args();

    mirakc_core::tracing_ext::init_tracing(&opt.log_format);

    let config = mirakc_core::config::load(&opt.config);

    let fs = TimeshiftFilesystem::new(config);

    let options = vec![
        fuser::MountOption::FSName("mirakc-timeshift".to_string()),
        fuser::MountOption::RO,
        fuser::MountOption::NoAtime,
    ];

    // The auto-unmount option does NOT work properly when the process terminates by signals like
    // SIGTERM.  Because the process terminates before fuser::Session<FS>::drop() is called.
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
