use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use notify_debouncer_mini::new_debouncer;
use tower_http::services::ServeDir;
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

    // Serve dist/ with livereload injection
    let app = Router::new()
        .fallback_service(ServeDir::new(&output_dir))
        .layer(livereload);

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
    ]
    .iter()
    .map(|d| root.join(d))
    .chain(std::iter::once(root.join("kiln.toml")))
    .collect();

    let tx = rebuild_tx.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = res
                && !events.is_empty() {
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
