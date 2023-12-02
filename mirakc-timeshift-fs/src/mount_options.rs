// Based-On: https://github.com/cberner/fuser/blob/v0.12.0/src/mnt/mount_options.rs
// SPDX-License-Identifier: MIT

use fuser::MountOption;

fn from_str(s: &str) -> MountOption {
    match s {
        "auto_unmount" => MountOption::AutoUnmount,
        "allow_other" => MountOption::AllowOther,
        "allow_root" => MountOption::AllowRoot,
        "default_permissions" => MountOption::DefaultPermissions,
        "dev" => MountOption::Dev,
        "nodev" => MountOption::NoDev,
        "suid" => MountOption::Suid,
        "nosuid" => MountOption::NoSuid,
        "ro" => MountOption::RO,
        "rw" => MountOption::RW,
        "exec" => MountOption::Exec,
        "noexec" => MountOption::NoExec,
        "atime" => MountOption::Atime,
        "noatime" => MountOption::NoAtime,
        "dirsync" => MountOption::DirSync,
        "sync" => MountOption::Sync,
        "async" => MountOption::Async,
        x if x.starts_with("fsname=") => MountOption::FSName(x[7..].into()),
        x if x.starts_with("subtype=") => MountOption::Subtype(x[8..].into()),
        x => MountOption::CUSTOM(x.into()),
    }
}

// Format option to be passed to libfuse or kernel
#[cfg(test)]
fn option_to_string(option: &MountOption) -> String {
    match option {
        MountOption::FSName(name) => format!("fsname={}", name),
        MountOption::Subtype(subtype) => format!("subtype={}", subtype),
        MountOption::CUSTOM(value) => value.to_string(),
        MountOption::AutoUnmount => "auto_unmount".to_string(),
        MountOption::AllowOther => "allow_other".to_string(),
        // AllowRoot is implemented by allowing everyone access and then restricting to
        // root + owner within fuser
        MountOption::AllowRoot => "allow_other".to_string(),
        MountOption::DefaultPermissions => "default_permissions".to_string(),
        MountOption::Dev => "dev".to_string(),
        MountOption::NoDev => "nodev".to_string(),
        MountOption::Suid => "suid".to_string(),
        MountOption::NoSuid => "nosuid".to_string(),
        MountOption::RO => "ro".to_string(),
        MountOption::RW => "rw".to_string(),
        MountOption::Exec => "exec".to_string(),
        MountOption::NoExec => "noexec".to_string(),
        MountOption::Atime => "atime".to_string(),
        MountOption::NoAtime => "noatime".to_string(),
        MountOption::DirSync => "dirsync".to_string(),
        MountOption::Sync => "sync".to_string(),
        MountOption::Async => "async".to_string(),
    }
}

/// Parses mount command args.
///
/// Input: ["suid", "ro,nodev,noexec", "sync"]
/// Output Ok([Suid, RO, NoDev, NoExec, Sync])
pub(crate) fn parse_options_from_args(args: &[String]) -> Vec<MountOption> {
    let mut out = vec![];
    for opt in args.iter() {
        for x in opt.split(',') {
            out.push(from_str(x))
        }
    }
    out
}

#[cfg(test)]
mod test {
    use super::*;
    use test_log::test;

    #[test]
    fn option_round_trip() {
        use super::MountOption::*;
        for x in [
            FSName("Blah".to_owned()),
            Subtype("Bloo".to_owned()),
            CUSTOM("bongos".to_owned()),
            AllowOther,
            AutoUnmount,
            DefaultPermissions,
            Dev,
            NoDev,
            Suid,
            NoSuid,
            RO,
            RW,
            Exec,
            NoExec,
            Atime,
            NoAtime,
            DirSync,
            Sync,
            Async,
        ]
        .iter()
        {
            assert_eq!(*x, from_str(option_to_string(x).as_ref()))
        }
    }

    #[test]
    fn test_parse_options() {
        use super::MountOption::*;

        assert_eq!(parse_options_from_args(&[]), &[]);

        let out = parse_options_from_args(&[
            "suid".to_string(),
            "ro,nodev,noexec".to_string(),
            "sync".to_string(),
        ]);
        assert_eq!(out, [Suid, RO, NoDev, NoExec, Sync]);

        //assert!(parse_options_from_args(&[OsStr::new("-o")]).is_err());
        //assert!(parse_options_from_args(&[OsStr::new("not o")]).is_err());
        //assert!(parse_options_from_args(&[OsStr::from_bytes(b"-o\xc3\x28")]).is_err());
    }
}
