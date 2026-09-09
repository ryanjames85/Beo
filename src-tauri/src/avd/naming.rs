/// avdmanager only accepts `[A-Za-z0-9._-]` in AVD names — anything else
/// (spaces, emoji, punctuation) either gets rejected outright or produces a
/// device whose on-disk name silently diverges from what was typed. Beo's
/// "name it anything" UX promises free-text input, so this maps that input
/// into a valid name instead of forwarding it as-is to the CLI.
pub(super) fn sanitize_avd_name(raw: &str) -> Result<String, String> {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_underscore = false;
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
            last_was_underscore = c == '_';
        } else if !last_was_underscore {
            out.push('_');
            last_was_underscore = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    let truncated: String = trimmed.chars().take(60).collect();
    if truncated.is_empty() {
        return Err("Device name needs at least one letter or number.".into());
    }
    if contains_blocked_word(&truncated) {
        return Err("That name isn't allowed — please choose something else.".into());
    }
    Ok(truncated)
}

// Deliberately small and word-boundary-matched rather than a substring scan
// — a substring check would block innocent names for containing a bad
// word as a fragment (e.g. "classic" contains "ass"), which is the classic
// failure mode of naive profanity filters. Checked against whole tokens of
// the *sanitized* name (already split on any non [A-Za-z0-9._-] character),
// so "bad_word" is checked as ["bad", "word"], not as one long string.
const BLOCKED_WORDS: &[&str] = &[
    "fuck", "shit", "bitch", "asshole", "cunt", "nigger", "nigga", "faggot", "retard", "whore",
    "slut", "dick", "piss", "cock", "pussy", "bastard",
];

pub(super) fn contains_blocked_word(sanitized_name: &str) -> bool {
    sanitized_name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| BLOCKED_WORDS.contains(&token.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_avd_name_replaces_disallowed_chars() {
        assert_eq!(sanitize_avd_name("My tablet").unwrap(), "My_tablet");
        assert_eq!(
            sanitize_avd_name("weird name!! @@ ##").unwrap(),
            "weird_name"
        );
    }

    #[test]
    fn sanitize_avd_name_collapses_runs_and_trims_edges() {
        // '.', '-', '_' are all allowed characters, so only leading/trailing
        // *underscores* get trimmed — a leading/trailing '.' is left as-is,
        // since it's just as valid mid-name as at the edges.
        assert_eq!(sanitize_avd_name("  __leading").unwrap(), "leading");
        assert_eq!(sanitize_avd_name("trailing__  ").unwrap(), "trailing");
        assert_eq!(sanitize_avd_name("a   b").unwrap(), "a_b");
    }

    #[test]
    fn sanitize_avd_name_allows_dots_dashes_underscores() {
        assert_eq!(
            sanitize_avd_name("my.device-1_test").unwrap(),
            "my.device-1_test"
        );
    }

    #[test]
    fn sanitize_avd_name_rejects_empty_result() {
        assert!(sanitize_avd_name("!!! @@@ ###").is_err());
        assert!(sanitize_avd_name("   ").is_err());
    }

    #[test]
    fn sanitize_avd_name_truncates_to_60_chars() {
        let long = "a".repeat(100);
        assert_eq!(sanitize_avd_name(&long).unwrap().len(), 60);
    }

    #[test]
    fn sanitize_avd_name_blocks_profanity_after_sanitizing() {
        // "shit device" sanitizes to "shit_device" before the blocklist
        // check runs — this confirms the check happens on the sanitized
        // form, not the raw input.
        assert!(sanitize_avd_name("fuck_this").is_err());
        assert!(sanitize_avd_name("shit device").is_err());
    }

    #[test]
    fn contains_blocked_word_matches_whole_tokens_only() {
        assert!(contains_blocked_word("fuck_device"));
        assert!(contains_blocked_word("my_shit_phone"));
        assert!(!contains_blocked_word("classic_assistant")); // contains "ass" as a substring, not a token
        assert!(!contains_blocked_word("my_tablet"));
    }

    #[test]
    fn contains_blocked_word_is_case_insensitive() {
        assert!(contains_blocked_word("FUCK"));
        assert!(contains_blocked_word("FuCk_device"));
    }
}
