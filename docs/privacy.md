---
title: "Privacy Policy"
permalink: /privacy/
---

# Privacy Policy

**Last updated: August 15, 2026**

This policy describes what data the **We** Discord bot (the "Bot")
processes, why, and what happens to it. The Bot is a private,
non-commercial application operated by its administrator (the
"Operator"). The short version: **the Bot stores nothing**. It keeps no
database and writes no member data to disk; Discord itself remains the
only record of your membership.

## 1. What the Bot processes

To do its one job — keeping every manageable member of a single,
explicitly configured Discord server (the "Server") under one shared
server nickname — the Bot receives the following data from Discord's API,
for that Server only:

- numeric identifiers: the Server (guild) ID, member user IDs, and role IDs;
- member usernames as included by Discord in member data;
- current server nicknames and role assignments of members;
- membership events for the Server (member joined, member updated).

The Bot requests only the Discord *Server Members* gateway intent in
addition to basic guild data. It does **not** request the Message Content
intent: it cannot see the content of any message. It processes no email
addresses, no IP addresses of members, and no messages.

## 2. What the Bot does with it

The data above is used solely to decide whether a member's server nickname
differs from the configured shared name and, if so, to change that
nickname through Discord's API. That is the Bot's only processing purpose.

## 3. Storage and retention

- **No database, no files.** The Bot keeps member data only in volatile
  memory while running, just long enough to act on it. When the process
  stops, that memory is gone.
- **Logs.** The Bot writes operational logs to its host system. Log
  entries may contain numeric guild IDs and user IDs (for example, "this
  member could not be renamed"), but not nicknames' text, usernames, or
  any message content. Logs are kept on the Operator's host under the
  host's normal log rotation and are not shared.
- **Secrets.** The Bot's own access token is supplied by the environment
  and is never logged.

## 4. Sharing

The Bot shares data with no one. Its only network communication is with
Discord's own API (gateway and REST), which is where all the data comes
from in the first place. There are no analytics, no advertising, no
third-party services, and no public endpoint of any kind.

Discord's handling of your data is governed by Discord's own
[Privacy Policy](https://discord.com/privacy).

## 5. Scope

The Bot operates in exactly one Server, configured by the Operator. If it
is ever present in any other server, it ignores that server entirely: no
data from other servers is acted upon.

## 6. Your choices and rights

- Your account-wide Discord display name is never touched; only your
  nickname *within the one Server* is managed.
- If you do not want your nickname managed, ask a Server administrator to
  place your role above the Bot's role or to remove the Bot, or leave the
  Server. Leaving the Server ends all processing of your data by the Bot
  immediately — the Bot holds nothing about you once you are gone.
- You may request information about, or deletion of, log entries
  referencing your user ID by contacting the Operator. Depending on your
  jurisdiction, you may have further statutory rights (such as access,
  rectification, or erasure); requests are honoured to the extent the
  minimal data described above exists at all.

## 7. Children

The Bot is usable only inside Discord and adds no data collection beyond
what is described above. Use of Discord itself is subject to Discord's own
age requirements.

## 8. Changes to this policy

This policy may be revised from time to time; the current version is
published on this page with the date of the last revision shown above.
Material changes will be announced in the Server.

## 9. Contact

Privacy questions and requests can be sent to the Operator at
`itops <AT> menhera dot ad dot jp`, or raised through the Server's
administrators.
