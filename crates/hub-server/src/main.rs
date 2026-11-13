use hub_app::{App, Config};
use hub_server::{build_router, modules};

fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    if let Err(error) = start() {
        eprintln!("team-hub: {error}");
        std::process::exit(1);
    }
}

fn start() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;

    // Request work and batch computations share one pool; blocking calls to other systems
    // never run on it.
    rayon::ThreadPoolBuilder::new()
        .thread_name(|index| format!("hub-worker-{index}"))
        .num_threads(std::thread::available_parallelism().map(|count| count.get().max(4)).unwrap_or(4))
        .build_global()?;

    let app = App::new(config.clone(), modules())?;
    app.start_modules();

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind((config.bind.as_str(), config.port)).await?;
        tracing::info!("team-hub listening on http://{}", listener.local_addr()?);

        axum::serve(listener, build_router(app))
            .with_graceful_shutdown(stop_signal())
            .await?;

        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

/// How long requests in flight get to finish once the process is asked to stop. The live
/// streams of open pages never end by themselves, so without a limit it would never exit.
const DRAIN: std::time::Duration = std::time::Duration::from_secs(10);

/// Resolves when the process is asked to stop: Ctrl-C, or the SIGTERM a container gets when
/// it is replaced. Requests in flight are then finished, within `DRAIN`, before the server exits.
async fn stop_signal() {
    asked_to_stop().await;
    tracing::info!("stopping");

    tokio::spawn(async {
        tokio::time::sleep(DRAIN).await;
        std::process::exit(0);
    });
}

async fn asked_to_stop() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }

            return;
        }
    }

    let _ = tokio::signal::ctrl_c().await;
}
