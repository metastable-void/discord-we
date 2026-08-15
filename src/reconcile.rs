//! The rule, kept pure and testable:
//!
//! > Everyone who can become "We" becomes "We".
//! > Anyone who ceases to be "We" becomes "We" again.

use std::cmp::Reverse;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileDecision {
    /// Wrong guild, or a member the bot must not / cannot touch.
    Ignore,
    /// The member already carries the name.
    AlreadyCorrect,
    /// The member must become the name again.
    SetNickname,
}

pub fn decide(
    event_guild_id: u64,
    target_guild_id: u64,
    manageable: bool,
    current_nick: Option<&str>,
    desired_nick: &str,
) -> ReconcileDecision {
    if event_guild_id != target_guild_id {
        return ReconcileDecision::Ignore;
    }
    if !manageable {
        return ReconcileDecision::Ignore;
    }
    if current_nick == Some(desired_nick) {
        ReconcileDecision::AlreadyCorrect
    } else {
        ReconcileDecision::SetNickname
    }
}

/// A member's standing in the role hierarchy: Discord orders roles by
/// position, and breaks position ties in favour of the *lower* (older) role
/// ID, hence the [`Reverse`].
pub type RoleRank = (u16, Reverse<u64>);

pub fn role_rank(position: u16, role_id: u64) -> RoleRank {
    (position, Reverse(role_id))
}

/// Role-hierarchy manageability. Discord additionally requires the Manage
/// Nicknames permission; a missing permission surfaces as an HTTP 403 and is
/// handled where the request is made.
pub fn member_is_manageable(
    owner_id: u64,
    bot_user_id: u64,
    member_user_id: u64,
    bot_best_role: RoleRank,
    member_best_role: RoleRank,
) -> bool {
    if member_user_id == bot_user_id {
        // The bot renames itself through the Change Nickname path instead.
        return false;
    }
    if member_user_id == owner_id {
        return false;
    }
    bot_best_role > member_best_role
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUILD: u64 = 1000;
    const OTHER_GUILD: u64 = 2000;

    #[test]
    fn wrong_guild_is_ignored() {
        assert_eq!(
            decide(OTHER_GUILD, GUILD, true, Some("Someone"), "We"),
            ReconcileDecision::Ignore
        );
    }

    #[test]
    fn unmanageable_member_is_ignored() {
        assert_eq!(
            decide(GUILD, GUILD, false, Some("Someone"), "We"),
            ReconcileDecision::Ignore
        );
    }

    #[test]
    fn correct_nickname_needs_no_update() {
        assert_eq!(
            decide(GUILD, GUILD, true, Some("We"), "We"),
            ReconcileDecision::AlreadyCorrect
        );
    }

    #[test]
    fn missing_nickname_is_updated() {
        assert_eq!(
            decide(GUILD, GUILD, true, None, "We"),
            ReconcileDecision::SetNickname
        );
    }

    #[test]
    fn different_nickname_is_updated() {
        assert_eq!(
            decide(GUILD, GUILD, true, Some("Me"), "We"),
            ReconcileDecision::SetNickname
        );
    }

    #[test]
    fn overridden_name_changes_the_target() {
        assert_eq!(
            decide(GUILD, GUILD, true, Some("We"), "Us"),
            ReconcileDecision::SetNickname
        );
        assert_eq!(
            decide(GUILD, GUILD, true, Some("Us"), "Us"),
            ReconcileDecision::AlreadyCorrect
        );
    }

    const OWNER: u64 = 1;
    const BOT: u64 = 2;
    const MEMBER: u64 = 3;

    #[test]
    fn owner_is_never_manageable() {
        assert!(!member_is_manageable(
            OWNER,
            BOT,
            OWNER,
            role_rank(10, 50),
            role_rank(0, 40)
        ));
    }

    #[test]
    fn bot_itself_is_not_managed_through_this_path() {
        assert!(!member_is_manageable(
            OWNER,
            BOT,
            BOT,
            role_rank(10, 50),
            role_rank(10, 50)
        ));
    }

    #[test]
    fn higher_bot_role_can_manage() {
        assert!(member_is_manageable(
            OWNER,
            BOT,
            MEMBER,
            role_rank(5, 50),
            role_rank(4, 40)
        ));
    }

    #[test]
    fn equal_or_higher_member_role_cannot_be_managed() {
        assert!(!member_is_manageable(
            OWNER,
            BOT,
            MEMBER,
            role_rank(5, 50),
            role_rank(5, 50)
        ));
        assert!(!member_is_manageable(
            OWNER,
            BOT,
            MEMBER,
            role_rank(4, 50),
            role_rank(5, 40)
        ));
    }

    #[test]
    fn position_ties_break_towards_the_older_role() {
        // Same position: the lower (older) role ID sits higher in the guild.
        assert!(member_is_manageable(
            OWNER,
            BOT,
            MEMBER,
            role_rank(5, 100),
            role_rank(5, 200)
        ));
        assert!(!member_is_manageable(
            OWNER,
            BOT,
            MEMBER,
            role_rank(5, 200),
            role_rank(5, 100)
        ));
    }
}
