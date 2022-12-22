use super::*;

use std::path::Path;
use std::task::Context;
use std::task::Poll;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::response::Response;
use futures::future::BoxFuture;
use tower::Service;
use tower_http::services::ServeDir;
use tower_http::services::ServeFile;

use crate::config::MountConfig;

const DEFAULT_FOLDER_VIEW_HTML_TEMPLATE_STR: &'static str =
    include_str!("default_folder_view.html.mustache");

pub(super) fn mount_entries<T, E, R, S>(
    config: &Config,
    router: Router<Arc<AppState<T, E, R, S>>>,
) -> Router<Arc<AppState<T, E, R, S>>>
where
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    let template = load_template(config);
    let mut router = router;
    for (mount_point, mount) in config.server.mounts.iter() {
        router = mount_entry(template.clone(), router, mount_point, mount);
    }
    router
}

fn load_template(config: &Config) -> Arc<mustache::Template> {
    Arc::new(config.server.folder_view_template_path.as_ref()
        .and_then(|path| {
            match mustache::compile_path(path) {
                Ok(template) => {
                    tracing::info!(?path, "Use user-specified folder view template");
                    Some(template)
                }
                Err(err) => {
                    tracing::warn!(%err, ?path, "Failed to load user-specified folder view template, use the default template");
                    None
                }
            }
        })
        .unwrap_or_else(|| {
            tracing::info!("Use the default folder view template");
            mustache::compile_str(DEFAULT_FOLDER_VIEW_HTML_TEMPLATE_STR).unwrap()
        })
    )
}

fn mount_entry<T, E, R, S>(
    template: Arc<mustache::Template>,
    router: Router<Arc<AppState<T, E, R, S>>>,
    mount_point: &str,
    mount: &MountConfig,
) -> Router<Arc<AppState<T, E, R, S>>>
where
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    if mount.path.is_dir() {
        tracing::info!(mount_point, ?mount.path, ?mount.index, mount.listing, "Mount directory");
        router.nest(mount_point, mount::mount_dir(template, mount_point, mount))
    } else if mount.path.is_file() {
        tracing::info!(mount_point, ?mount.path, ?mount.index, mount.listing, "Mount file");
        router.nest(mount_point, mount::mount_file(mount))
    } else {
        tracing::warn!(mount_point, ?mount.path, "Only regular files and directries can be mounted");
        router
    }
}

fn mount_dir<T, E, R, S>(
    template: Arc<mustache::Template>,
    mount_point: &str,
    mount: &MountConfig,
) -> Router<Arc<AppState<T, E, R, S>>>
where
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    let service = ServeDir::new(&mount.path)
        .append_index_html_on_directories(false)
        .fallback(ListingService::new(template, mount_point, mount));
    Router::new().fallback_service(routing::get_service(service).handle_error(convert_error))
}

fn mount_file<T, E, R, S>(mount: &MountConfig) -> Router<Arc<AppState<T, E, R, S>>>
where
    T: Send + Sync + 'static,
    E: Send + Sync + 'static,
    R: Send + Sync + 'static,
    S: Send + Sync + 'static,
{
    let service = ServeFile::new_with_mime(&mount.path, &mime::TEXT_HTML_UTF_8);
    Router::new().fallback_service(routing::get_service(service).handle_error(convert_error))
}

#[derive(Clone)]
struct ListingService {
    template: Arc<mustache::Template>,
    mount_point: String,
    config: MountConfig,
}

impl ListingService {
    fn new(template: Arc<mustache::Template>, mount_point: &str, mount: &MountConfig) -> Self {
        ListingService {
            template,
            mount_point: mount_point.to_string(),
            config: mount.clone(),
        }
    }
}

impl Service<Request<Body>> for ListingService {
    type Response = Response;
    type Error = std::io::Error;
    // `BoxFuture` is a type alias for `Pin<Box<dyn Future + Send + 'a>>`
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let url_path = req
            .uri()
            .path()
            .trim_start_matches('/')
            .trim_end_matches('/');
        match (self.config.index.as_ref(), self.config.listing) {
            (Some(index_file), _) => {
                let path = self.config.path.join(url_path).join(index_file);
                if path.is_file() {
                    let relpath = path.strip_prefix(&self.config.path).unwrap();
                    let index_path = format!("{}/{}", self.mount_point, relpath.to_str().unwrap());
                    tracing::debug!(index_path, "Redirect to the index file");
                    Box::pin(async move { Ok(Redirect::to(&index_path).into_response()) })
                } else {
                    tracing::warn!(?path, "File not found");
                    Box::pin(async { Ok(StatusCode::NOT_FOUND.into_response()) })
                }
            }
            (_, true) => {
                let path = self.config.path.join(url_path);
                if path.is_dir() {
                    let base_url = if url_path.is_empty() {
                        self.mount_point.clone()
                    } else {
                        format!("{}/{}", self.mount_point, url_path)
                    };
                    let template = self.template.clone();
                    tracing::debug!(?path, "Render the folder view");
                    Box::pin(async move {
                        Ok(render_folder_view(&path, &base_url, &template)
                            .await
                            .into_response())
                    })
                } else {
                    tracing::warn!(?path, "Directory not found");
                    Box::pin(async { Ok(StatusCode::NOT_FOUND.into_response()) })
                }
            }
            _ => Box::pin(async { Ok(StatusCode::NOT_FOUND.into_response()) }),
        }
    }
}

async fn render_folder_view<P: AsRef<Path>>(
    path: P,
    base_url: &str,
    template: &mustache::Template,
) -> Result<Html<String>, crate::error::Error> {
    // Correct entries in the directory.
    let data = mustache::MapBuilder::new()
        .insert_vec("entries", |mut builder| {
            for entry in iter_entries(&path) {
                let filename = entry.file_name().to_str();
                let metadata = entry.metadata();
                if let (Some(name), Ok(metadata)) = (filename, metadata) {
                    builder = builder.push_map(|mut builder| {
                        builder = builder
                            .insert_str("name", name)
                            .insert_str("size", metadata.len().to_string());
                        builder = if metadata.is_dir() {
                            // This is a dirty hack...
                            // ServeDir doesn't work properly if a URL of a directory
                            // doesn't end with '/'.
                            builder.insert_str("url", format!("{}/{}/", base_url, name))
                        } else {
                            builder.insert_str("url", format!("{}/{}", base_url, name))
                        };
                        if let Ok(created) = metadata.created() {
                            if let Ok(unix_time) = created.duration_since(UNIX_EPOCH) {
                                builder =
                                    builder.insert_str("created", unix_time.as_secs().to_string());
                            }
                        }
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(unix_time) = modified.duration_since(UNIX_EPOCH) {
                                builder =
                                    builder.insert_str("modified", unix_time.as_secs().to_string());
                            }
                        }
                        builder
                    });
                }
            }
            builder
        })
        .build();

    Ok(Html(template.render_data_to_string(&data)?))
}

fn iter_entries<P: AsRef<Path>>(path: P) -> impl Iterator<Item = walkdir::DirEntry> {
    walkdir::WalkDir::new(path)
        .max_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.depth() != 0) // remove the root
}

async fn convert_error(err: std::io::Error) -> Error {
    err.into()
}

// <coverage:exclude>
#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use std::fmt::Write as _;
    use std::io::Write as _;

    #[test]
    fn test_load_template() {
        // default template
        let config = Config::default();
        let template = load_template(&config);
        let result = template.render_to_string(&());
        assert_matches!(result, Ok(result) => {
            assert_eq!(result, "<ul>\n</ul>\n");
        });

        // File not found, use default template
        let mut config = Config::default();
        config.server.folder_view_template_path = Some("/no-such-file".into());
        let template = load_template(&config);
        let result = template.render_to_string(&());
        assert_matches!(result, Ok(result) => {
            assert_eq!(result, "<ul>\n</ul>\n");
        });

        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("template.mustache");
        let mut file = std::fs::File::create(&path).unwrap();
        write!(&mut file, "dummy").unwrap();
        drop(file);
        let mut config = Config::default();
        config.server.folder_view_template_path = Some(path);
        let template = load_template(&config);
        let result = template.render_to_string(&());
        assert_matches!(result, Ok(result) => {
            assert_eq!(result, "dummy");
        });
    }

    #[tokio::test]
    async fn test_render_folder_view() {
        let template = mustache::compile_str(
            "{{#entries}}\
             {{name}},{{size}},{{url}},{{created}},{{modified}}
             {{/entries}}",
        )
        .unwrap();

        let path = env!("CARGO_MANIFEST_DIR");
        let base_url = "BASEURL";

        let mut expected = String::new();
        for entry in iter_entries(&path) {
            let name = entry.file_name().to_str().unwrap();
            let metadata = entry.metadata().unwrap();
            let size = metadata.len();
            let url = if metadata.is_dir() {
                format!("{base_url}/{name}/")
            } else {
                format!("{base_url}/{name}")
            };
            let created = metadata
                .created()
                .ok()
                .map(|time| {
                    time.duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        .to_string()
                })
                .unwrap_or_default();
            let modified = metadata
                .modified()
                .ok()
                .map(|time| {
                    time.duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        .to_string()
                })
                .unwrap_or_default();
            write!(&mut expected, "{name},{size},{url},{created},{modified}\n").unwrap();
        }

        let result = render_folder_view(path, base_url, &template).await;
        assert_matches!(result, Ok(Html(html)) => {
            assert_eq!(html, expected);
        });
    }
}
// </coverage:exclude>
