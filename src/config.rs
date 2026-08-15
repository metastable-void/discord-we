//! Configuration from environment variables, including the resolution of the
//! name everyone becomes.

use std::time::Duration;

use anyhow::{Context as _, bail};
use serenity::model::id::GuildId;
use tracing::warn;

/// The name everyone becomes when nothing else is configured.
pub const FALLBACK_NAME: &str = "We";

/// Discord rejects guild nicknames longer than 32 characters.
const MAX_NICKNAME_CHARS: usize = 32;

#[derive(Debug, Clone)]
pub struct Config {
    pub token: String,
    pub guild_id: GuildId,
    /// The nickname enforced on every manageable member: `We`, unless
    /// overridden through the `OUR_NAME` environment variable.
    pub our_name: String,
    /// Optional low-frequency periodic full-guild sweep; `None` disables it.
    pub reconcile_interval: Option<Duration>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("DISCORD_TOKEN").context("DISCORD_TOKEN is not set")?;
        if token.trim().is_empty() {
            bail!("DISCORD_TOKEN is empty");
        }

        let guild_id = std::env::var("TARGET_GUILD_ID").context("TARGET_GUILD_ID is not set")?;
        let guild_id: u64 = guild_id
            .trim()
            .parse()
            .context("TARGET_GUILD_ID must be a numeric guild ID (snowflake)")?;
        if guild_id == 0 {
            bail!("TARGET_GUILD_ID must be non-zero");
        }

        let our_name = resolve_our_name(std::env::var("OUR_NAME").ok());

        let reconcile_interval = match std::env::var("RECONCILE_INTERVAL_SECONDS") {
            Ok(seconds) if !seconds.trim().is_empty() => {
                let seconds: u64 = seconds
                    .trim()
                    .parse()
                    .context("RECONCILE_INTERVAL_SECONDS must be an integer number of seconds")?;
                (seconds > 0).then(|| Duration::from_secs(seconds))
            }
            _ => None,
        };

        Ok(Self {
            token,
            guild_id: GuildId::new(guild_id),
            our_name,
            reconcile_interval,
        })
    }
}

/// Why a configured name cannot be used as a Discord guild nickname.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRejection {
    /// More than 32 characters.
    TooLong,
    /// Contains control characters.
    ControlCharacters,
    /// Would render as visibly empty (only whitespace and invisible code
    /// points), which the Discord client forbids.
    VisiblyEmpty,
}

/// Resolves `OUR_NAME`: unset or empty means the canonical `We`; a value that
/// Discord would reject as a nickname is refused with a warning and also falls
/// back to `We`.
pub fn resolve_our_name(raw: Option<String>) -> String {
    let Some(name) = raw.filter(|name| !name.is_empty()) else {
        return FALLBACK_NAME.to_owned();
    };
    match validate_nickname(&name) {
        Ok(()) => name,
        Err(reason) => {
            warn!(
                ?reason,
                configured = ?name,
                fallback = FALLBACK_NAME,
                "OUR_NAME is not usable as a Discord nickname; falling back"
            );
            FALLBACK_NAME.to_owned()
        }
    }
}

pub fn validate_nickname(name: &str) -> Result<(), NameRejection> {
    if name.chars().count() > MAX_NICKNAME_CHARS {
        return Err(NameRejection::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(NameRejection::ControlCharacters);
    }
    if !name.chars().any(is_visible) {
        return Err(NameRejection::VisiblyEmpty);
    }
    Ok(())
}

/// Code points that render as blank in a name even though they are not ASCII
/// whitespace: zero-width and joiner characters, bidi controls, fillers,
/// variation selectors, the braille blank, and similar format characters.
fn is_invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c,
            '\u{00AD}'                  // soft hyphen
            | '\u{034F}'                // combining grapheme joiner
            | '\u{061C}'                // arabic letter mark
            | '\u{115F}' | '\u{1160}'   // hangul choseong/jungseong fillers
            | '\u{17B4}' | '\u{17B5}'   // khmer inherent vowels
            | '\u{180B}'..='\u{180E}'   // mongolian variation selectors, vowel separator
            | '\u{200B}'..='\u{200F}'   // zero-width space/joiners, directional marks
            | '\u{202A}'..='\u{202E}'   // bidi embedding controls
            | '\u{2060}'..='\u{2064}'   // word joiner, invisible operators
            | '\u{2066}'..='\u{206F}'   // bidi isolates, deprecated format characters
            | '\u{2800}'                // braille pattern blank
            | '\u{3164}'                // hangul filler
            | '\u{FE00}'..='\u{FE0F}'   // variation selectors
            | '\u{FEFF}'                // zero-width no-break space
            | '\u{FFA0}'                // halfwidth hangul filler
            | '\u{FFF9}'..='\u{FFFB}'   // interlinear annotation controls
            | '\u{1D173}'..='\u{1D17A}' // musical score formatting controls
            | '\u{E0000}'..='\u{E007F}' // tag characters
        )
}

fn is_visible(c: char) -> bool {
    !is_invisible(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_name_defaults_to_we() {
        assert_eq!(resolve_our_name(None), "We");
        assert_eq!(resolve_our_name(Some(String::new())), "We");
    }

    #[test]
    fn valid_names_are_kept() {
        assert_eq!(resolve_our_name(Some("Us".to_owned())), "Us");
        assert_eq!(resolve_our_name(Some("私たち".to_owned())), "私たち");
        let exactly_32 = "W".repeat(32);
        assert_eq!(resolve_our_name(Some(exactly_32.clone())), exactly_32);
        // Visible emoji held together by zero-width joiners are fine.
        assert_eq!(
            resolve_our_name(Some("👨\u{200D}👩\u{200D}👧".to_owned())),
            "👨\u{200D}👩\u{200D}👧"
        );
    }

    #[test]
    fn visibly_empty_names_fall_back_to_we() {
        for name in [
            "   ",
            "\u{200B}\u{200B}",
            "\u{3164}",
            "\u{2800}\u{2800}",
            "\u{FEFF}",
        ] {
            assert_eq!(
                resolve_our_name(Some(name.to_owned())),
                "We",
                "name: {name:?}"
            );
            assert_eq!(validate_nickname(name), Err(NameRejection::VisiblyEmpty));
        }
    }

    #[test]
    fn overlong_names_fall_back_to_we() {
        let too_long = "W".repeat(33);
        assert_eq!(validate_nickname(&too_long), Err(NameRejection::TooLong));
        assert_eq!(resolve_our_name(Some(too_long)), "We");
    }

    #[test]
    fn control_characters_fall_back_to_we() {
        assert_eq!(
            validate_nickname("a\nb"),
            Err(NameRejection::ControlCharacters)
        );
        assert_eq!(resolve_our_name(Some("a\nb".to_owned())), "We");
    }
}
