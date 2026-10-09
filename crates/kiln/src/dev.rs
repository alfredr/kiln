use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use anyhow::Context;
use axum::Router;
use axum::http::{HeaderValue, header};
use notify_debouncer_mini::new_debouncer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_livereload::LiveReloadLayer;

use crate::build;
use crate::config::Config;

pub async fn dev(
    root: &Path,
    config: &Config,
    port: u16,
    include_drafts: bool,
) -> anyhow::Result<()> {
    let output_dir = root.join(&config.build.output_dir);

    // Initial build
    eprintln!("performing initial build...");
    build::build(root, config, include_drafts)?;

    // Set up livereload
    let livereload = LiveReloadLayer::new();
    let reloader = livereload.reloader();

    // Serve dist/ with livereload injection. `no-cache` makes the browser
    // revalidate every file on reload; otherwise it heuristically caches
    // stylesheets and keeps showing stale CSS after a rebuild.
    let app = Router::new()
        .fallback_service(ServeDir::new(&output_dir))
        .layer(livereload)
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ));

    // Rebuild channel — bounded(1) coalesces rapid changes
    let (rebuild_tx, rebuild_rx) = mpsc::sync_channel::<()>(1);

    // Spawn rebuild thread
    let rebuild_root = root.to_path_buf();
    let rebuild_drafts = include_drafts;
    std::thread::spawn(move || {
        rebuild_loop(rebuild_rx, &rebuild_root, rebuild_drafts, reloader);
    });

    // Set up debounced file watcher
    // NOTE: Watch paths are set at startup. If content/template/static dirs
    // change in kiln.toml, a restart is required to pick up the new paths.
    let watch_dirs: Vec<_> = [
        &config.build.content_dir,
        &config.build.template_dir,
        &config.build.static_dir,
        &config.build.data_dir,
    ]
    .iter()
    .map(|d| root.join(d))
    .chain(std::iter::once(root.join("kiln.toml")))
    .collect();

    let tx = rebuild_tx.clone();
    let mut changes = ChangeFilter::baseline(&watch_dirs);
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            let Ok(events) = res else { return };
            let mut changed = false;
            for event in &events {
                if changes.changed(&event.path) {
                    eprintln!("  changed: {}", event.path.display());
                    changed = true;
                }
            }
            if changed {
                let _ = tx.try_send(());
            }
        },
    )?;

    for dir in &watch_dirs {
        if !dir.exists() {
            continue;
        }
        let mode = if dir.is_dir() {
            notify_debouncer_mini::notify::RecursiveMode::Recursive
        } else {
            notify_debouncer_mini::notify::RecursiveMode::NonRecursive
        };
        debouncer
            .watcher()
            .watch(dir, mode)
            .with_context(|| format!("failed to watch {}", dir.display()))?;
    }

    // Start server
    let addr = format!("127.0.0.1:{port}");
    eprintln!("dev server running at http://localhost:{port}");
    eprintln!("watching for changes... (ctrl+c to stop)");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Filters file events down to real changes. On macOS, FSEvents keeps
/// reporting static files every time a build copies them, even though their
/// contents are untouched; acting on those events rebuilds (and reloads the
/// browser) in an endless loop. A path counts as changed only when its
/// modification time or size differs from the last time we saw it.
struct ChangeFilter {
    seen: HashMap<PathBuf, (SystemTime, u64)>,
}

impl ChangeFilter {
    fn baseline(roots: &[PathBuf]) -> Self {
        let seen = roots
            .iter()
            .flat_map(|root| walkdir::WalkDir::new(root).into_iter().filter_map(Result::ok))
            .filter_map(|entry| {
                let path = entry.into_path();
                fingerprint(&path).map(|f| (path, f))
            })
            .collect();
        Self { seen }
    }

    fn changed(&mut self, path: &Path) -> bool {
        let now = fingerprint(path);
        if now == self.seen.get(path).copied() {
            return false;
        }
        match now {
            Some(f) => self.seen.insert(path.to_path_buf(), f),
            None => self.seen.remove(path),
        };
        true
    }
}

fn fingerprint(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn rebuild_loop(
    rx: mpsc::Receiver<()>,
    root: &Path,
    include_drafts: bool,
    reloader: tower_livereload::Reloader,
) {
    while rx.recv().is_ok() {
        eprintln!("\nchange detected, rebuilding...");

        let config = match Config::load(root) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  config error: {e}");
                continue;
            }
        };

        match build::build(root, &config, include_drafts) {
            Ok(()) => {
                eprintln!("rebuild complete");
                reloader.reload();
            }
            Err(e) => {
                eprintln!("  build error: {e}");
            }
        }

        // Drain pending signals
        while rx.try_recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_filter_ignores_events_without_real_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let css = tmp.path().join("main.css");
        std::fs::write(&css, "a {}").unwrap();

        let mut filter = ChangeFilter::baseline(&[tmp.path().to_path_buf()]);
        assert!(!filter.changed(&css), "untouched file reported as changed");

        std::fs::write(&css, "a { color: red }").unwrap();
        assert!(filter.changed(&css));
        assert!(!filter.changed(&css), "same change reported twice");

        // A temp file that appeared and vanished between events is not a change.
        assert!(!filter.changed(&tmp.path().join("main.css.tmp.123")));

        let new = tmp.path().join("new.css");
        std::fs::write(&new, "b {}").unwrap();
        assert!(filter.changed(&new));

        std::fs::remove_file(&css).unwrap();
        assert!(filter.changed(&css));
    }
}
