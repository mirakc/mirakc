mod filesystem;

use clap;
use fuser;

use mirakc_core;
use mirakc_core::error::Error;
use crate::filesystem::TimeshiftFilesystem;

fn main() -> Result<(), Error> {
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
        .arg(clap::Arg::with_name("log-format")
             .long("log-format")
             .value_name("FORMAT")
             .env("MIRAKC_LOG_FORMAT")
             .takes_value(true)
             .possible_values(&["text", "json"])
             .default_value("text")
             .help("Logging format"))
        .arg(clap::Arg::with_name("MOUNT_POINT")
            .index(1)
            .required(true)
            .help("Path to the mount point"))
        .get_matches();

    mirakc_core::tracing_ext::init_tracing(args.value_of("log-format").unwrap());

    let config_path = args.value_of("config").expect(
        "--config option or MIRAKC_CONFIG environment must be specified");

    let config = mirakc_core::config::load(config_path);

    let fs = TimeshiftFilesystem::new(config);

    let options = vec![
        fuser::MountOption::FSName("mirakc-timeshift".to_string()),
        fuser::MountOption::RO,
        fuser::MountOption::NoAtime,
        fuser::MountOption::AllowOther,
        fuser::MountOption::AutoUnmount,  // not work properly, see comments below.
    ];

    let mount_point = args.value_of("MOUNT_POINT").unwrap();

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
    Ok(fuser::mount2(fs, mount_point, &options)?)
}
