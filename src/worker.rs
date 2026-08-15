//! Bounded reconciliation: a coalescing queue, a couple of workers, and the
//! full-guild sweep. All REST writes happen here, never in gateway callbacks.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serenity::Error as SerenityError;
use serenity::all::{Cache, EditMember, Http, UserId};
use serenity::http::{HttpError, StatusCode};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use crate::discord::{App, is_manageable};
use crate::reconcile::{ReconcileDecision, decide};

/// Discord returns at most 1000 members per list-members page.
const MEMBERS_PAGE: u64 = 1000;
const MAX_ATTEMPTS: u32 = 3;
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);

/// A bounded work queue of member IDs, coalescing duplicates: a member
/// already waiting is not queued a second time.
pub struct Queue {
    tx: mpsc::Sender<UserId>,
    pending: Mutex<HashSet<UserId>>,
    accepting: AtomicBool,
}

impl Queue {
    pub fn bounded(capacity: usize) -> (Self, mpsc::Receiver<UserId>) {
        let (tx, rx) = mpsc::channel(capacity);
        let queue = Self {
            tx,
            pending: Mutex::new(HashSet::new()),
            accepting: AtomicBool::new(true),
        };
        (queue, rx)
    }

    pub async fn enqueue(&self, user_id: UserId) {
        if !self.accepting.load(Ordering::Relaxed) {
            return;
        }
        if !self.lock_pending().insert(user_id) {
            return;
        }
        if self.tx.send(user_id).await.is_err() {
            self.lock_pending().remove(&user_id);
        }
    }

    /// Called on shutdown: new work is refused, workers drain what they must.
    pub fn stop_accepting(&self) {
        self.accepting.store(false, Ordering::Relaxed);
    }

    /// A worker took the member out of the queue; later deviations may
    /// enqueue the same member again.
    fn finish(&self, user_id: UserId) {
        self.lock_pending().remove(&user_id);
    }

    fn lock_pending(&self) -> MutexGuard<'_, HashSet<UserId>> {
        // Poisoning would require a panic while holding the lock; the set
        // stays usable either way.
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Spawns the bounded worker pool. Workers stop on the shutdown signal or
/// when the queue closes.
pub fn spawn_workers(
    count: usize,
    http: Arc<Http>,
    app: Arc<App>,
    rx: mpsc::Receiver<UserId>,
    shutdown: watch::Receiver<bool>,
) -> Vec<JoinHandle<()>> {
    let rx = Arc::new(tokio::sync::Mutex::new(rx));
    (0..count)
        .map(|worker| {
            let http = http.clone();
            let app = app.clone();
            let rx = rx.clone();
            let mut shutdown = shutdown.clone();
            tokio::spawn(async move {
                loop {
                    let next = {
                        let mut rx = rx.lock().await;
                        tokio::select! {
                            biased;
                            _ = shutdown.changed() => None,
                            item = rx.recv() => item,
                        }
                    };
                    let Some(user_id) = next else { break };
                    app.queue.finish(user_id);
                    enforce_nickname(&http, &app, user_id).await;
                }
                debug!(worker, "reconciliation worker stopped");
            })
        })
        .collect()
}

enum FailureClass {
    /// Role hierarchy, ownership, or missing permission: expected, final.
    Unmanageable,
    /// The member (or the guild) is gone: final.
    Gone,
    /// Server or network trouble: worth a bounded retry.
    Transient,
    /// Anything else: final, and worth a loud log line.
    Permanent,
}

fn classify(err: &SerenityError) -> FailureClass {
    match err {
        SerenityError::Http(HttpError::UnsuccessfulRequest(response)) => {
            match response.status_code {
                StatusCode::FORBIDDEN => FailureClass::Unmanageable,
                StatusCode::NOT_FOUND => FailureClass::Gone,
                // Serenity's ratelimiter normally absorbs 429s; if one leaks
                // through, backing off is still the right answer.
                StatusCode::TOO_MANY_REQUESTS => FailureClass::Transient,
                status if status.is_server_error() => FailureClass::Transient,
                _ => FailureClass::Permanent,
            }
        }
        SerenityError::Http(_) => FailureClass::Transient,
        _ => FailureClass::Permanent,
    }
}

/// One REST write, with bounded retries for transient failures only.
async fn enforce_nickname(http: &Arc<Http>, app: &App, user_id: UserId) {
    let guild_id = app.cfg.guild_id;
    let mut backoff = INITIAL_BACKOFF;
    for attempt in 1..=MAX_ATTEMPTS {
        let edit = EditMember::new().nickname(app.cfg.our_name.as_str());
        match guild_id.edit_member(http, user_id, edit).await {
            Ok(_) => {
                info!(
                    event = "nickname_enforced",
                    guild_id = guild_id.get(),
                    user_id = user_id.get(),
                    "member became \"We\" (the configured name) again"
                );
                return;
            }
            Err(err) => match classify(&err) {
                FailureClass::Unmanageable => {
                    info!(
                        event = "member_unmanageable",
                        guild_id = guild_id.get(),
                        user_id = user_id.get(),
                        error = %err,
                        "cannot manage member (role hierarchy, ownership, or missing permission)"
                    );
                    return;
                }
                FailureClass::Gone => {
                    debug!(
                        event = "member_gone",
                        guild_id = guild_id.get(),
                        user_id = user_id.get(),
                        "member left before reconciliation"
                    );
                    return;
                }
                FailureClass::Permanent => {
                    error!(
                        event = "discord_api_error",
                        operation = "edit_member",
                        guild_id = guild_id.get(),
                        user_id = user_id.get(),
                        error = %err,
                        "permanent error while enforcing nickname"
                    );
                    return;
                }
                FailureClass::Transient => {
                    if attempt == MAX_ATTEMPTS {
                        error!(
                            event = "discord_api_error",
                            operation = "edit_member",
                            guild_id = guild_id.get(),
                            user_id = user_id.get(),
                            error = %err,
                            "giving up after repeated transient failures"
                        );
                        return;
                    }
                    warn!(
                        event = "discord_api_error",
                        operation = "edit_member",
                        guild_id = guild_id.get(),
                        user_id = user_id.get(),
                        attempt,
                        error = %err,
                        "transient error; retrying"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff *= 2;
                }
            },
        }
    }
}

#[derive(Default)]
struct SweepStats {
    scanned: usize,
    enqueued: usize,
    unmanageable: usize,
}

/// Full-guild reconciliation: pages through the member list and enqueues
/// every member who is not yet the name. Used after startup / re-identify and
/// by the optional periodic repair sweep.
pub async fn sweep_guild(http: Arc<Http>, cache: Arc<Cache>, app: Arc<App>) {
    if !app.begin_sweep() {
        debug!("guild reconciliation already in progress; skipping");
        return;
    }
    let guild_id = app.cfg.guild_id;
    info!(
        event = "guild_reconciliation_started",
        guild_id = guild_id.get(),
        "starting full guild reconciliation"
    );
    let outcome = run_sweep(&http, &cache, &app).await;
    app.end_sweep();
    match outcome {
        Ok(stats) => info!(
            event = "guild_reconciliation_completed",
            guild_id = guild_id.get(),
            scanned = stats.scanned,
            enqueued = stats.enqueued,
            unmanageable = stats.unmanageable,
            "full guild reconciliation finished"
        ),
        Err(err) => warn!(
            event = "discord_api_error",
            operation = "member_sweep",
            guild_id = guild_id.get(),
            error = %err,
            "guild reconciliation aborted; the next sweep will repair"
        ),
    }
}

async fn run_sweep(
    http: &Arc<Http>,
    cache: &Cache,
    app: &App,
) -> Result<SweepStats, SerenityError> {
    let guild_id = app.cfg.guild_id;
    sync_own_member(http, app).await;

    let bot_id = app.bot_user_id();
    let mut stats = SweepStats::default();
    let mut after: Option<UserId> = None;
    loop {
        let page = guild_id.members(http, Some(MEMBERS_PAGE), after).await?;
        for member in &page {
            stats.scanned += 1;
            let manageable = is_manageable(cache, app, member.user.id, &member.roles);
            match decide(
                guild_id.get(),
                guild_id.get(),
                manageable,
                member.nick.as_deref(),
                &app.cfg.our_name,
            ) {
                ReconcileDecision::SetNickname => {
                    stats.enqueued += 1;
                    app.queue.enqueue(member.user.id).await;
                }
                ReconcileDecision::Ignore => {
                    if Some(member.user.id) != bot_id {
                        stats.unmanageable += 1;
                        debug!(
                            event = "member_unmanageable",
                            user_id = member.user.id.get(),
                            "skipping unmanageable member during sweep"
                        );
                    }
                }
                ReconcileDecision::AlreadyCorrect => {}
            }
        }
        if (page.len() as u64) < MEMBERS_PAGE {
            break;
        }
        after = page.last().map(|member| member.user.id);
    }
    Ok(stats)
}

/// Refreshes the bot's own role list (for hierarchy checks) and, where the
/// optional Change Nickname permission allows, makes the bot itself carry the
/// name too.
async fn sync_own_member(http: &Arc<Http>, app: &App) {
    let guild_id = app.cfg.guild_id;
    match guild_id.current_user_member(http).await {
        Ok(me) => {
            app.set_bot_roles(me.roles.clone());
            if me.nick.as_deref() != Some(app.cfg.our_name.as_str()) {
                match guild_id.edit_nickname(http, Some(&app.cfg.our_name)).await {
                    Ok(()) => info!(
                        event = "nickname_enforced",
                        guild_id = guild_id.get(),
                        target = "self",
                        "the bot itself became the name"
                    ),
                    Err(err) if matches!(classify(&err), FailureClass::Unmanageable) => debug!(
                        target = "self",
                        "Change Nickname not granted; leaving own nickname as-is"
                    ),
                    Err(err) => warn!(
                        event = "discord_api_error",
                        operation = "edit_nickname",
                        guild_id = guild_id.get(),
                        error = %err,
                        "could not set own nickname"
                    ),
                }
            }
        }
        Err(err) => warn!(
            event = "discord_api_error",
            operation = "current_user_member",
            guild_id = guild_id.get(),
            error = %err,
            "could not fetch own guild membership"
        ),
    }
}
