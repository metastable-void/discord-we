# Coding Agent Prompt: “We” — Guild-Specific Discord Nickname Enforcer

## Objective

Build a private, guild-specific Discord bot whose artistic purpose is to make every manageable member appear under the same guild nickname:

> **We**

The bot must:

1. On startup / initial guild synchronization, set every manageable guild member’s nickname to `We`.
2. When a new member joins, set their nickname to `We`.
3. When a member changes their guild nickname to anything other than `We`, change it back to `We`.
4. Recover cleanly from reconnects, missed events, and downtime by reconciling guild state.
5. Operate only in one explicitly configured Discord guild.
6. Require no public HTTP(S) endpoint for normal operation.

The preferred implementation language is **Rust**. Use a mature Discord library such as **Serenity** unless current ecosystem limitations make that impractical. Only fall back to **Node.js + discord.js** if there is a concrete technical reason to do so.

---

## Important Discord Semantics

The bot cannot change users’ account-wide Discord display names. It must enforce the **guild-specific nickname** instead.

Treat the desired invariant as:

```text
For every manageable member in TARGET_GUILD_ID:
    guild nickname == "We"
```

Members that Discord does not allow the bot to manage because of role hierarchy or ownership constraints must be skipped safely and logged.

The bot itself may optionally use the nickname `We` as well.

---

## Architecture

Use Discord’s Gateway plus REST API:

```text
Discord Gateway
    |
    | outbound WebSocket
    v
Bot process
    |
    | event handlers
    v
Reconciliation / work queue
    |
    | outbound HTTPS
    v
Discord REST API
```

Do **not** add a public web server, callback endpoint, reverse proxy, TLS listener, webhook receiver, or database unless there is a demonstrated need.

Normal bot installation and Gateway operation should not require any inbound HTTP(S) endpoint.

### Required event coverage

Handle at least:

- guild availability / initial ready state
- guild member add
- guild member update
- Gateway reconnect / resume edge cases where reconciliation is appropriate

Use the minimum Gateway intents necessary. The implementation is expected to need:

- `GUILDS`
- `GUILD_MEMBERS`

Document that the Server Members privileged intent must be enabled for the application.

---

## Configuration

Read configuration from environment variables.

Required:

```text
DISCORD_TOKEN=
TARGET_GUILD_ID=
```

Optional:

```text
TARGET_NICKNAME=We
LOG_LEVEL=info
RECONCILE_INTERVAL_SECONDS=
```

`TARGET_NICKNAME` should default to exactly:

```text
We
```

The process must refuse to enforce nicknames outside `TARGET_GUILD_ID`.

If it unexpectedly appears in another guild, either:

- ignore that guild completely, or
- leave it automatically.

Choose one behavior, document it, and keep the implementation simple.

Never log the bot token.

---

## Discord Permissions

The bot should request only the permissions it actually needs.

Expected permissions:

- **Manage Nicknames** — required to change other members’ guild nicknames
- **Change Nickname** — optional, only if the bot should rename itself to `We`

Do not require Administrator.

Document Discord role hierarchy behavior clearly:

- the bot cannot rename members whose highest role is at or above the bot’s highest role;
- the guild owner is not generally manageable by the bot;
- those cases are expected limitations, not fatal errors.

---

## Core Reconciliation Logic

Implement one central operation conceptually equivalent to:

```rust
async fn reconcile_member(member: &Member) -> Result<()> {
    if member.guild_id != TARGET_GUILD_ID {
        return Ok(());
    }

    if !member_is_manageable(member) {
        return Ok(());
    }

    if member.nick.as_deref() != Some(TARGET_NICKNAME) {
        set_guild_nickname(member.user.id, TARGET_NICKNAME).await?;
    }

    Ok(())
}
```

Exact library APIs will differ.

All event handlers should converge on this reconciliation behavior rather than duplicating nickname-setting logic.

### Idempotency

Only send a nickname update when the current nickname differs from the desired nickname.

This is important because the bot’s own REST update will itself result in a member-update event.

The event caused by the bot must therefore naturally become a no-op.

---

## Initial Guild Sweep

After the bot is ready and the target guild is available:

1. obtain the complete member list;
2. inspect each member;
3. enqueue reconciliation for each manageable member;
4. do not launch an unbounded number of concurrent REST calls.

Support guilds large enough that member enumeration may require chunking or pagination according to the selected Discord library.

---

## Work Queue / Concurrency

Do not perform unlimited REST writes directly inside Gateway callbacks.

Implement a small bounded reconciliation mechanism.

A suitable design is:

```text
Gateway event
    -> enqueue (guild_id, user_id)
    -> deduplicate/coalesce repeated requests
    -> bounded workers
    -> fetch/check current state if needed
    -> PATCH nickname only if required
```

Requirements:

- bounded concurrency;
- graceful handling of Discord rate limits;
- duplicate member updates may be coalesced;
- retry transient Discord/network failures with reasonable bounded backoff;
- do not retry permanent permission/hierarchy failures indefinitely.

Use the Discord library’s rate-limit handling rather than reimplementing Discord’s full rate-limit protocol.

Keep the queue implementation proportionate to the project. This is a small bot, not a distributed system.

---

## Reconciliation After Downtime

Gateway events can be missed during downtime or unsuccessful resume scenarios.

Provide at least one recovery mechanism:

- full reconciliation after startup / fresh Gateway session;
- optionally, a low-frequency periodic full guild reconciliation.

If implementing periodic reconciliation, keep the interval configurable and conservative.

The Gateway event handlers remain the fast path; the periodic sweep is only eventual-consistency repair.

---

## Error Handling

Classify failures usefully.

Expected non-fatal cases include:

- member is above bot in role hierarchy;
- member is guild owner;
- missing nickname-management permission;
- member left before queued update was processed;
- guild/member disappeared during reconciliation.

Log these without crashing the process.

Unexpected API errors should include enough structured context to diagnose:

- guild ID;
- user ID;
- operation;
- Discord error category/status where available.

Never emit secrets or excessive personal data.

---

## Observability

Use structured logging.

For Rust, prefer `tracing`.

Useful events:

```text
bot_connected
guild_reconciliation_started
guild_reconciliation_completed
nickname_change_detected
nickname_enforced
member_unmanageable
discord_api_error
gateway_reconnected
```

Avoid logging every successful no-op at normal log levels.

At startup, print a concise configuration summary without the token.

Example:

```text
target_guild=123...
target_nickname="We"
periodic_reconciliation=true
```

---

## Graceful Shutdown

Handle process termination cleanly.

On SIGINT/SIGTERM:

- stop accepting new queued work;
- allow a brief orderly queue shutdown where practical;
- close the Discord client cleanly;
- exit without corrupting state.

There is no persistent application state that needs to be saved.

---

## Persistence

Do not introduce a database.

Discord is the source of truth.

The desired state consists only of configuration:

```text
target guild
desired nickname = "We"
```

All member state can be reconstructed from Discord.

---

## Security

Follow least privilege.

Requirements:

- token only via environment/secret manager;
- no token in repository;
- provide `.env.example`, never a real `.env`;
- no Administrator permission;
- ignore all non-target guilds;
- avoid unnecessary message-content permissions;
- do not request Message Content intent;
- no public HTTP listener by default.

---

## Rust Implementation Preference

Prefer:

```text
Rust
Tokio
Serenity
tracing
thiserror and/or anyhow where appropriate
```

Use current stable releases that are mutually compatible at implementation time. Verify the current Discord and library APIs instead of relying on stale examples.

Suggested module shape:

```text
src/
  main.rs
  config.rs
  discord.rs
  reconcile.rs
  worker.rs
  error.rs
```

This is only a suggestion. Avoid needless abstraction if fewer modules make the code clearer.

### Rust quality requirements

- `cargo fmt` clean
- `cargo clippy --all-targets --all-features` clean, apart from explicitly justified exceptions
- meaningful error propagation
- no `unwrap()`/`expect()` in normal runtime paths unless logically impossible and documented
- async code must not block the Tokio runtime
- bounded channels rather than unbounded queues unless clearly justified

---

## Node.js Fallback

Only use Node.js if the Rust implementation is demonstrably blocked by missing or immature Discord functionality.

If falling back:

```text
Node.js
TypeScript
discord.js
```

Use TypeScript rather than plain JavaScript.

Preserve the same architecture and constraints:

- Gateway events
- REST nickname changes
- target-guild allowlist
- bounded reconciliation
- no public HTTP endpoint
- no database

Document exactly why Rust was rejected before implementing the fallback.

---

## Suggested Repository Deliverables

Produce:

```text
README.md
Cargo.toml
Cargo.lock
.env.example
.gitignore
src/...
```

If Node fallback is used instead:

```text
README.md
package.json
package-lock.json (or equivalent lockfile)
tsconfig.json
.env.example
.gitignore
src/...
```

Also include an optional deployment example using one simple long-running-process method such as:

- systemd; or
- Docker.

Do not require Kubernetes.

---

## README Requirements

The README must explain:

### What the bot does

Every manageable member of one Discord guild is assigned the guild nickname:

```text
We
```

and any later deviation is reconciled back to:

```text
We
```

### Discord setup

Explain:

1. create Discord application;
2. create bot user;
3. disable public bot installation if appropriate for the private installation;
4. enable Server Members privileged intent;
5. invite/install into the target guild;
6. grant Manage Nicknames;
7. move bot role high enough in the guild role hierarchy;
8. optionally grant Change Nickname for the bot’s own nickname;
9. configure `DISCORD_TOKEN` and `TARGET_GUILD_ID`.

Do not suggest granting Administrator.

### Networking

Explicitly state:

> This bot does not require a public HTTP(S) endpoint. It establishes outbound connections to Discord’s Gateway and REST API.

### Known Discord limitations

Document members the bot cannot rename because of role hierarchy or ownership.

### Running

Provide concise development and production commands.

---

## Tests

Add tests where they provide real value.

At minimum, separate enough logic from Discord transport to test the decision:

```text
nickname == "We"     -> no update
nickname != "We"     -> update
nickname == null     -> update
wrong guild          -> ignore
unmanageable member  -> ignore
```

Do not overbuild mocks of the entire Discord API.

If the library architecture makes direct unit testing of event objects awkward, create a small pure decision function such as:

```rust
enum ReconcileDecision {
    Ignore,
    AlreadyCorrect,
    SetNickname,
}
```

and test that thoroughly.

---

## Acceptance Criteria

The project is complete when all of the following are true:

- [ ] Rust implementation works with a current stable Discord library, or a documented technical reason justifies the Node.js fallback.
- [ ] No public HTTP(S) endpoint is required.
- [ ] The bot connects through Discord Gateway.
- [ ] On startup, all manageable target-guild members are reconciled to `We`.
- [ ] New members are reconciled to `We`.
- [ ] A user changing their guild nickname away from `We` causes it to be restored.
- [ ] The bot does not repeatedly PATCH a member already named `We`.
- [ ] Non-target guilds are never modified.
- [ ] Role-hierarchy failures do not crash or create infinite retries.
- [ ] REST writes use bounded concurrency.
- [ ] Reconnect/startup recovery performs reconciliation.
- [ ] No database is required.
- [ ] No Administrator permission is required.
- [ ] Server Members intent is documented.
- [ ] Secrets are not committed or logged.
- [ ] README contains installation and deployment instructions.
- [ ] Formatting/linting/tests pass.

---

## Implementation Process

Work autonomously and make reasonable engineering decisions without asking for approval on trivial choices.

Before coding:

1. verify current Discord API requirements for guild member nickname modification and Gateway member events;
2. verify the current stable Rust Discord ecosystem;
3. choose Serenity unless another mature Rust library is clearly preferable;
4. note any Discord API/library behavior that changes the architecture described above.

Then:

1. scaffold the project;
2. implement configuration;
3. implement the Discord client and required intents;
4. implement reconciliation logic;
5. implement bounded work processing;
6. implement startup/member-add/member-update handling;
7. add recovery reconciliation;
8. add structured logs;
9. add tests;
10. write README and deployment example;
11. run formatter, linter, and tests;
12. review the result for unnecessary permissions, services, and complexity.

Favor the smallest reliable implementation.

The artistic rule should remain legible in the code:

```text
Everyone who can become "We" becomes "We".
Anyone who ceases to be "We" becomes "We" again.
```
