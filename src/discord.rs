//! Gateway event handling: every event converges on the same reconciliation
//! decision and, when needed, enqueues the member for the workers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use serenity::all::{
    Cache, Context, EventHandler, Guild, GuildMemberUpdateEvent, Member, Ready, ResumedEvent,
    RoleId, UserId,
};
use serenity::async_trait;
use tracing::{debug, info};

use crate::config::Config;
use crate::reconcile::{ReconcileDecision, RoleRank, decide, member_is_manageable, role_rank};
use crate::worker::{Queue, sweep_guild};

/// Shared state: configuration, the work queue, and the little the bot needs
/// to remember about itself.
pub struct App {
    pub cfg: Config,
    pub queue: Queue,
    /// 0 until the READY event arrives.
    bot_user_id: AtomicU64,
    bot_roles: Mutex<Vec<RoleId>>,
    sweeping: AtomicBool,
}

impl App {
    pub fn new(cfg: Config, queue: Queue) -> Self {
        Self {
            cfg,
            queue,
            bot_user_id: AtomicU64::new(0),
            bot_roles: Mutex::new(Vec::new()),
            sweeping: AtomicBool::new(false),
        }
    }

    pub fn bot_user_id(&self) -> Option<UserId> {
        match self.bot_user_id.load(Ordering::Relaxed) {
            0 => None,
            id => Some(UserId::new(id)),
        }
    }

    pub fn set_bot_user_id(&self, id: UserId) {
        self.bot_user_id.store(id.get(), Ordering::Relaxed);
    }

    pub fn bot_roles(&self) -> Vec<RoleId> {
        self.lock_bot_roles().clone()
    }

    pub fn set_bot_roles(&self, roles: Vec<RoleId>) {
        *self.lock_bot_roles() = roles;
    }

    /// Returns false if a sweep is already running.
    pub fn begin_sweep(&self) -> bool {
        !self.sweeping.swap(true, Ordering::SeqCst)
    }

    pub fn end_sweep(&self) {
        self.sweeping.store(false, Ordering::SeqCst);
    }

    fn lock_bot_roles(&self) -> std::sync::MutexGuard<'_, Vec<RoleId>> {
        // Poisoning would require a panic while holding the lock; the data is
        // a plain Vec and stays usable either way.
        self.bot_roles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Role-hierarchy manageability using the cached guild (roles and owner are
/// always cached under the GUILDS intent). If the guild is somehow not cached
/// the member is treated as manageable and the API stays the final judge.
pub fn is_manageable(cache: &Cache, app: &App, user_id: UserId, member_roles: &[RoleId]) -> bool {
    let Some(bot_id) = app.bot_user_id() else {
        return false;
    };
    if user_id == bot_id {
        return false;
    }
    let Some(guild) = cache.guild(app.cfg.guild_id) else {
        return true;
    };
    let bot_rank = best_role_rank(&guild, &app.bot_roles());
    let member_rank = best_role_rank(&guild, member_roles);
    member_is_manageable(
        guild.owner_id.get(),
        bot_id.get(),
        user_id.get(),
        bot_rank,
        member_rank,
    )
}

/// The member's highest role, falling back to @everyone (position 0, ID equal
/// to the guild ID).
fn best_role_rank(guild: &Guild, role_ids: &[RoleId]) -> RoleRank {
    let everyone = role_rank(0, guild.id.get());
    role_ids
        .iter()
        .filter_map(|id| guild.roles.get(id))
        .map(|role| role_rank(role.position, role.id.get()))
        .max()
        .map_or(everyone, |best| best.max(everyone))
}

pub struct Handler {
    pub app: std::sync::Arc<App>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, data: Ready) {
        self.app.set_bot_user_id(data.user.id);
        info!(
            event = "bot_connected",
            bot_user_id = data.user.id.get(),
            guilds = data.guilds.len(),
            "connected to the Discord gateway"
        );
    }

    async fn resume(&self, _ctx: Context, _event: ResumedEvent) {
        // On a successful resume Discord replays the missed events; a failed
        // resume re-identifies and re-delivers GUILD_CREATE, which triggers a
        // full sweep below.
        info!(event = "gateway_reconnected", "gateway session resumed");
    }

    async fn guild_create(&self, ctx: Context, guild: Guild, _is_new: Option<bool>) {
        let app = &self.app;
        if guild.id != app.cfg.guild_id {
            // Deliberate policy: guilds outside TARGET_GUILD_ID are ignored
            // completely (the bot stays but never acts there).
            info!(
                event = "foreign_guild_ignored",
                guild_id = guild.id.get(),
                "ignoring guild outside TARGET_GUILD_ID"
            );
            return;
        }

        // Seed our role list from the GUILD_CREATE payload so that hierarchy
        // checks work before the sweep's own REST refresh lands.
        if let Some(bot_id) = app.bot_user_id()
            && let Some(me) = guild.members.get(&bot_id)
        {
            app.set_bot_roles(me.roles.clone());
        }

        info!(
            event = "target_guild_available",
            guild_id = guild.id.get(),
            member_count = guild.member_count,
            "target guild available; scheduling full reconciliation"
        );
        tokio::spawn(sweep_guild(
            ctx.http.clone(),
            ctx.cache.clone(),
            app.clone(),
        ));
    }

    async fn guild_member_addition(&self, ctx: Context, member: Member) {
        let app = &self.app;
        if member.guild_id != app.cfg.guild_id {
            return;
        }
        let manageable = is_manageable(&ctx.cache, app, member.user.id, &member.roles);
        match decide(
            member.guild_id.get(),
            app.cfg.guild_id.get(),
            manageable,
            member.nick.as_deref(),
            &app.cfg.our_name,
        ) {
            ReconcileDecision::SetNickname => {
                info!(
                    event = "nickname_change_detected",
                    trigger = "member_join",
                    user_id = member.user.id.get(),
                    "new member; enqueueing reconciliation"
                );
                app.queue.enqueue(member.user.id).await;
            }
            ReconcileDecision::Ignore => {
                debug!(
                    event = "member_unmanageable",
                    user_id = member.user.id.get(),
                    "new member is not manageable; skipping"
                );
            }
            ReconcileDecision::AlreadyCorrect => {}
        }
    }

    async fn guild_member_update(
        &self,
        ctx: Context,
        _old_if_available: Option<Member>,
        _new: Option<Member>,
        event: GuildMemberUpdateEvent,
    ) {
        let app = &self.app;
        if event.guild_id != app.cfg.guild_id {
            return;
        }

        if app.bot_user_id() == Some(event.user.id) {
            app.set_bot_roles(event.roles.clone());
            if event.nick.as_deref() != Some(app.cfg.our_name.as_str()) {
                // Someone renamed the bot itself; take the name back. This
                // needs only Change Nickname and is idempotent: our own edit
                // re-arrives here with the correct nickname and does nothing.
                info!(
                    event = "nickname_change_detected",
                    trigger = "self",
                    "own nickname changed; restoring"
                );
                if let Err(err) = app
                    .cfg
                    .guild_id
                    .edit_nickname(&ctx.http, Some(&app.cfg.our_name))
                    .await
                {
                    debug!(
                        event = "member_unmanageable",
                        trigger = "self",
                        error = %err,
                        "could not restore own nickname (Change Nickname missing?)"
                    );
                }
            }
            return;
        }

        let manageable = is_manageable(&ctx.cache, app, event.user.id, &event.roles);
        match decide(
            event.guild_id.get(),
            app.cfg.guild_id.get(),
            manageable,
            event.nick.as_deref(),
            &app.cfg.our_name,
        ) {
            ReconcileDecision::SetNickname => {
                info!(
                    event = "nickname_change_detected",
                    trigger = "member_update",
                    user_id = event.user.id.get(),
                    "nickname deviates; enqueueing reconciliation"
                );
                app.queue.enqueue(event.user.id).await;
            }
            ReconcileDecision::Ignore => {
                debug!(
                    event = "member_unmanageable",
                    user_id = event.user.id.get(),
                    "updated member is not manageable; skipping"
                );
            }
            // Includes the member-update events caused by our own PATCH:
            // the nickname now matches, so the cycle ends here.
            ReconcileDecision::AlreadyCorrect => {}
        }
    }
}
