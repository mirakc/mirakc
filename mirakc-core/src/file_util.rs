use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

pub fn save_json<D, P>(data: D, path: P) -> bool
where
    D: Serialize,
    P: AsRef<Path>,
    P: std::fmt::Debug,
{
    // Serialize records in advance in order to improve error traceability.
    let buf = match serde_json::to_vec(&data) {
        Ok(buf) => {
            tracing::debug!(?path, "Serialized data successfully");
            buf
        }
        Err(err) => {
            tracing::error!(%err, ?path, "Failed to serialize data");
            return false;
        }
    };

    save_data(&buf, path)
}

pub fn save_data<P>(data: &[u8], path: P) -> bool
where
    P: AsRef<Path>,
    P: std::fmt::Debug,
{
    // Write the data to a temporal file called <path>.new.
    let new_path = append_extension(&path, "new");
    {
        let mut file = match std::fs::File::create(&new_path) {
            Ok(file) => {
                tracing::debug!(?path, "Created <path>.new file for saving data");
                file
            }
            Err(err) => {
                tracing::error!(%err, ?path, "Failed to create <path>.new file");
                return false;
            }
        };
        match file.write_all(&data) {
            Ok(_) => {
                tracing::debug!(
                    nwritten = data.len(),
                    ?path,
                    "Wrote data to <path>.new file"
                );
            }
            Err(err) => {
                tracing::error!(%err, ?path, "Failed to write data to <path>.new");
                return false;
            }
        }
        match file.sync_all() {
            Ok(_) => {
                tracing::debug!(?path, "Sync <path>.new file to disk");
            }
            Err(err) => {
                tracing::error!(%err, ?path, "Failed to sync <path>.new file to disk");
                return false;
            }
        }
    }

    // Then, rename it to the original.
    match std::fs::rename(&new_path, &path) {
        Ok(_) => {
            tracing::debug!(?path, "Renamed <path>.new to <path>");
        }
        Err(err) => {
            tracing::error!(%err, ?path, "Failed to rename <path>.new to <path>");
            return false;
        }
    }

    true
}

fn append_extension<P, S>(path: P, ext: S) -> PathBuf
where
    P: AsRef<Path>,
    S: AsRef<OsStr>,
{
    let path = path.as_ref();
    match path.extension() {
        Some(last_ext) => {
            let mut last_ext = last_ext.to_os_string();
            last_ext.push(".");
            last_ext.push(ext);
            path.with_extension(last_ext)
        }
        None => path.with_extension(ext),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use tempfile::TempDir;
    use test_log::test;

    #[test]
    fn test_save_data() {
        let temp_dir = TempDir::new().unwrap();

        let path = temp_dir.path().join("test");
        let new_path = append_extension(&path, "new");
        assert!(!path.exists());
        assert!(!new_path.exists());

        let ok = save_data(b"foo", &path);
        assert!(ok);
        assert!(path.exists());
        assert_matches!(std::fs::read(&path), Ok(data) => {
            assert!(data == b"foo");
        });
        assert!(!new_path.exists());

        let ok = save_data(b"bar", &path);
        assert!(ok);
        assert!(path.exists());
        assert_matches!(std::fs::read(&path), Ok(data) => {
            assert!(data == b"bar");
        });
        assert!(!new_path.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_save_data_file_create_fails() {
        let path = Path::new("/sys/cannot_write");
        let new_path = append_extension(&path, "new");
        assert!(!path.exists());
        assert!(!new_path.exists());

        let ok = save_data(b"foo", &path);
        assert!(!ok);
        assert!(!path.exists());
        assert!(!new_path.exists());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn test_save_data_rename_fails() {
        // TODO: need allowing write, but preventing rename
    }

    #[test]
    fn test_append_extension() {
        assert_eq!(append_extension("foobar", "baz"), Path::new("foobar.baz"));
        assert_eq!(append_extension("foo.bar", "baz"), Path::new("foo.bar.baz"));
    }
}
