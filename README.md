# We

A private, guild-specific Discord bot with one artistic purpose: every
manageable member of one configured guild carries the same guild nickname —

> **We**

Any later deviation is reconciled back to **We**.

```text
Everyone who can become "We" becomes "We".
Anyone who ceases to be "We" becomes "We" again.
```

The bot changes **guild nicknames** only. It cannot (and does not try to)
change anyone's account-wide Discord display name, and it never acts outside
the one configured guild.

## How it works

- On startup (and whenever the target guild becomes available after a
  reconnect that re-identified), the bot sweeps the complete member list and
  reconciles every manageable member to the name.
- When a member joins, they are reconciled to the name.
- When a member changes their guild nickname to anything else, it is changed
  back.
- A member already carrying the name is never PATCHed again, so the bot's own
  updates converge instead of looping.
- All REST writes go through a small bounded, coalescing work queue (two
  workers, bounded channel), with bounded backoff for transient errors.
  Serenity's built-in rate-limit handling is used as-is.
- Optionally, a low-frequency periodic full sweep repairs anything missed
  during downtime. Gateway events remain the fast path.

Any guild other than `TARGET_GUILD_ID` is **ignored completely**: the bot
stays connected if invited elsewhere but never reads or changes anything
there.

## Networking

> This bot does not require a public HTTP(S) endpoint. It establishes
> outbound connections to Discord's Gateway (WebSocket) and REST API only.

No web server, webhook receiver, reverse proxy, TLS listener, or database is
involved. Discord itself is the source of truth; the only state is the
configuration.

## Configuration

Environment variables (see `.env.example`):

| Variable | Required | Meaning |
| --- | --- | --- |
| `DISCORD_TOKEN` | yes | Bot token. Never committed, never logged. |
| `TARGET_GUILD_ID` | yes | The only guild the bot enforces. |
| `OUR_NAME` | no | The name everyone becomes. Unset or empty means `We`. |
| `LOG_LEVEL` | no | `tracing` filter, default `info` (`RUST_LOG` also works). |
| `RECONCILE_INTERVAL_SECONDS` | no | Periodic repair sweep; unset or `0` disables it. |

`OUR_NAME` is validated against what Discord accepts as a guild nickname: a
value that would be **visibly empty** (only whitespace and invisible code
points such as zero-width spaces, the Hangul filler, or the braille blank),
longer than 32 characters, or containing control characters is rejected with
a warning and the bot falls back to `We`.

## Discord setup

1. Create an application at <https://discord.com/developers/applications>.
2. Add a bot user to it and copy the token.
3. This is a private bot: disable **Public Bot** on the bot page.
4. Enable the **Server Members Intent** (privileged) on the bot page — the
   bot cannot see member joins/updates without it.
5. Invite the bot into the target guild with the **Manage Nicknames**
   permission (`134217728`; use `201326592` to also include the optional
   Change Nickname):
   `https://discord.com/oauth2/authorize?client_id=<APP_ID>&scope=bot&permissions=134217728`
6. In the guild's role settings, move the bot's role **above** every role it
   should manage. Role hierarchy, not the permission alone, decides who is
   manageable.
7. Optionally also grant **Change Nickname** so the bot can rename itself to
   the name too.
8. Set `DISCORD_TOKEN` and `TARGET_GUILD_ID` in the environment.

Do **not** grant Administrator; the bot neither needs nor requests it.

## Known Discord limitations

Some members can never be renamed by a bot; these are expected and are
logged and skipped, not errors:

- the **guild owner**;
- any member whose **highest role is at or above** the bot's highest role;
- everyone, if the bot lacks the Manage Nicknames permission.

The bot renaming **itself** additionally needs the Change Nickname
permission; without it the bot simply keeps its own name.

## Running

Development:

```sh
cp .env.example .env       # fill in DISCORD_TOKEN and TARGET_GUILD_ID
set -a; . ./.env; set +a
cargo run
```

Production (static musl binary, no runtime dependencies):

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
# -> target/x86_64-unknown-linux-musl/release/discord-we (static-pie ELF)
```

The code is portable POSIX-ish Rust (Tokio + Serenity with rustls, no
OpenSSL, no C library assumptions beyond a working C toolchain for the
target); other Unix targets build the same way.

Checks:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
```

## Deployment

A hardened systemd unit is provided in
[deploy/discord-we.service](deploy/discord-we.service), including the
install commands. It runs the static binary as a dedicated system user with
an `EnvironmentFile` holding the token.

## Logs

Structured `tracing` logs, one event vocabulary throughout:
`bot_connected`, `guild_reconciliation_started`,
`guild_reconciliation_completed`, `nickname_change_detected`,
`nickname_enforced`, `member_unmanageable`, `discord_api_error`,
`gateway_reconnected`. Successful no-ops are not logged at `info`. The token
is never logged; at startup only the target guild, the effective name, and
whether periodic reconciliation is enabled are printed.
