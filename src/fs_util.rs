use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn modified<P>(path: P) -> Option<SystemTime>
where
    P: AsRef<Path>
{
    fs::metadata(path)
        .map(|metadata| metadata.modified().ok())
        .ok()
        .flatten()
}

pub fn unmodified_since<P>(path: P, datetime: Option<SystemTime>) -> bool
where
    P: AsRef<Path>
{
    match datetime {
        Some(datetime) => {
            match modified(path) {
                Some(modified) => modified < datetime,
                None => false,
            }
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unmodified_since() {
        let path = PathBuf::from(file!());
        let modified = modified(&path);
        assert!(!unmodified_since(&path, None));
        assert!(!unmodified_since(&path, modified));
        assert!(unmodified_since(&path, Some(SystemTime::now())));
    }
}
