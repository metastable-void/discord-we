//! discord-we — a private, guild-specific Discord bot.
//!
//! Everyone who can become "We" becomes "We".
//! Anyone who ceases to be "We" becomes "We" again.

mod config;
mod discord;
mod reconcile;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use serenity::all::{Cache, Client, GatewayIntents, Http};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::discord::{App, Handler};
use crate::worker::Queue;

const QUEUE_CAPACITY: usize = 1024;
const WORKER_COUNT: usize = 2;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env()?;
    info!(
        target_guild = cfg.guild_id.get(),
        target_nickname = %cfg.our_name,
        periodic_reconciliation = cfg.reconcile_interval.is_some(),
        "configuration loaded"
    );

    let (queue, rx) = Queue::bounded(QUEUE_CAPACITY);
    let app = Arc::new(App::new(cfg, queue));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // GUILD_MEMBERS is a privileged intent and must be enabled for the
    // application in the Discord developer portal.
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_MEMBERS;
    let mut client = Client::builder(&app.cfg.token, intents)
        .event_handler(Handler { app: app.clone() })
        .await
        .context("failed to build the Discord client")?;

    let workers = worker::spawn_workers(
        WORKER_COUNT,
        client.http.clone(),
        app.clone(),
        rx,
        shutdown_rx.clone(),
    );

    if let Some(every) = app.cfg.reconcile_interval {
        tokio::spawn(periodic_reconciliation(
            every,
            client.http.clone(),
            client.cache.clone(),
            app.clone(),
            shutdown_rx,
        ));
    }

    {
        let app = app.clone();
        let shard_manager = client.shard_manager.clone();
        tokio::spawn(async move {
            wait_for_termination().await;
            info!(event = "shutdown_requested", "termination signal received");
            app.queue.stop_accepting();
            shard_manager.shutdown_all().await;
        });
    }

    let run = client.start().await;

    // Reached on signal-initiated shutdown or on a fatal client error either
    // way, wind the workers down before leaving.
    app.queue.stop_accepting();
    let _ = shutdown_tx.send(true);
    drop(client);
    for handle in workers {
        let _ = handle.await;
    }
    info!("shutdown complete");
    run.context("the Discord client terminated with an error")
}

/// `LOG_LEVEL` takes precedence, then `RUST_LOG`, then plain `info`.
fn init_tracing() {
    let filter = match std::env::var("LOG_LEVEL") {
        Ok(level) if !level.trim().is_empty() => {
            EnvFilter::try_new(level.trim()).unwrap_or_else(|err| {
                eprintln!("invalid LOG_LEVEL ({err}); defaulting to info");
                EnvFilter::new("info")
            })
        }
        _ => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Optional eventual-consistency repair; the gateway handlers stay the fast
/// path. The first tick is skipped because GUILD_CREATE already triggers the
/// startup sweep.
async fn periodic_reconciliation(
    every: Duration,
    http: Arc<Http>,
    cache: Arc<Cache>,
    app: Arc<App>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut timer = tokio::time::interval(every);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    timer.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = timer.tick() => {}
        }
        if app.bot_user_id().is_none() {
            continue;
        }
        worker::sweep_guild(http.clone(), cache.clone(), app.clone()).await;
    }
}

async fn wait_for_termination() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "cannot listen for SIGTERM; handling SIGINT only");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
