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

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " ", env!("VERGEN_GIT_SHA"));

#[derive(Parser)]
#[command(author, about, version = VERSION)]
struct Opt {
    /// Path to a configuration file.
    ///
    /// The MIRAKC_CONFIG environment variable is used if this option is not
    /// specified.  Its value has to be an absolute path.
    ///
    /// YAML (*.yml or *.yaml) or TOML (*.toml) formats are supported.
    ///
    /// See docs/config.md for details of the YAML/TOML format.
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

    /// Prepend the start time field in the filename of each record.
    ///
    /// If this option is enabled, the filename will be formatted in:
    ///
    ///   <record.start_time>.<record.id>.<sanitized record.program.name>.m2ts
    ///
    /// Otherwise:
    ///
    ///   <record.id>.<sanitized record.program.name>.m2ts
    ///
    /// <record.start_time> is the start time in local time formatted in `%Y-%m-%d-%H-%M-%S`.
    ///
    /// <record.id> is the record ID formatted in 8 uppercase hexadecimal digits.
    ///
    /// This option is disabled by default for backward compatibility.
    #[arg(long, env = "MIRAKC_TIMESHIFT_FS_START_TIME_PREFIX")]
    start_time_prefix: bool,

    /// Logging format.
    #[arg(long, value_enum, env = "MIRAKC_LOG_FORMAT", default_value = "text")]
    log_format: LogFormat,

    /// Additional mount options.
    ///
    /// The following mount options will be added internally:
    /// subtype=mirakc-timeshift-fs,ro,noatime
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
        start_time_prefix: opt.start_time_prefix,
    };
    let fs = TimeshiftFilesystem::new(config, fs_config);

    let mut options = parse_options_from_args(&opt.options);
    options.push(fuser::MountOption::Subtype(
        "mirakc-timeshift-fs".to_string(),
    ));
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
