mod filesystem;

// `fuser::mount()` was deprecatged, but `fuser::mnt::mount_options::parse_options_from_args()`
// is still private.  So, there is no way to parse mount options specified in the command line...
// We copied mnt/mount_options.rs from cberner/fuser.
mod mount_options;

use std::path::PathBuf;

use clap::Parser;
use clap::ValueEnum;

use crate::filesystem::TimeshiftFilesystem;
use crate::filesystem::TimeshiftFilesystemConfig;
use crate::mount_options::parse_options_from_args;
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
    #[arg(short, long, env = "MIRAKC_CONFIG")]
    config: PathBuf,

    /// Owner's numeric UID.
    ///
    /// UID of the user running the process is used by default.
    #[arg(short, long, env = "MIRAKC_TIMESHIFT_FS_UID", default_value_t = unsafe { libc::getuid() })]
    uid: u32,

    /// Owner's numeric GID.
    ///
    /// GID of the user running the process is used by default.
    #[arg(short, long, env = "MIRAKC_TIMESHIFT_FS_GID", default_value_t = unsafe { libc::getgid() })]
    gid: u32,

    /// Logging format.
    #[arg(long, value_enum, env = "MIRAKC_LOG_FORMAT", default_value = "text")]
    log_format: LogFormat,

    /// Additional mount options.
    ///
    /// The following mount options will be added internally:
    /// fsname=mirakc-timeshift,ro,noatime
    #[arg(short, long, env = "MIRAKC_TIMESHIFT_FS_MOUNT_OPTIONS")]
    options: Vec<String>,

    /// Path to the mount point.
    #[arg(env = "MIRAKC_TIMESHIFT_FS_MOUNT_POINT", default_value = "/mnt")]
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

    let fs_config = TimeshiftFilesystemConfig {
        uid: opt.uid,
        gid: opt.gid,
    };
    let fs = TimeshiftFilesystem::new(config, fs_config);

    let mut options = parse_options_from_args(&opt.options);
    options.push(fuser::MountOption::FSName("mirakc-timeshift".to_string()));
    options.push(fuser::MountOption::RO);
    options.push(fuser::MountOption::NoAtime);

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
