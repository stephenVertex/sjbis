/// Heuristic: does this message look like it requires a human response?
///
/// Same logic as iMessage plugin — catches questions, requests, and binary choices.
/// Filters out short reactions, emojis, and pure punctuation.
pub fn looks_like_question(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();

    // Very short (just reactions, emojis, exclamations)
    if trimmed.len() < 4 {
        return false;
    }

    // Exact-match non-questions: standalone reactions/confirmations
    let non_questions_exact = [
        "lol", "lmao", "haha", "ha", "ok", "okay", "k", "kk", "mhm", "uh huh",
        "thanks", "ty", "np", "yw",
        "sure", "nice", "cool", "wow", "omg", "omfg", "wtf", "wth",
        "brb", "gtg", "ttyl", "bbl",
        "omw", "cya",
        "sg", "sgtm",
        "done",
        "yes", "no", "yep", "nope", "yeah", "nah", "yup", "nn",
        "congrats", "congratulations", "hbd",
        "gl", "hf",
        "rip", "f", "ffs",
        "huh", "uh", "um", "er",
    ];
    for pattern in &non_questions_exact {
        if lower == *pattern {
            return false;
        }
    }

    // Starts-with non-questions: longer phrases
    let non_questions_starts = [
        "on my way", "be there", "see you",
        "sounds good", "sounds great",
        "got it", "will do", "ok great", "perfect", "awesome",
        "happy birthday", "good luck", "have fun",
    ];
    for pattern in &non_questions_starts {
        if lower.starts_with(pattern) {
            return false;
        }
    }

    // Messages that are mostly punctuation / symbols / numbers
    let alphanumeric_count = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if alphanumeric_count < 3 {
        let question_starts = [
            "what", "when", "where", "why", "how", "who", "which",
            "can you", "could you", "would you", "will you", "did you", "do you",
            "are you", "is it", "should i", "shall we", "may i",
        ];
        let has_question_start = question_starts.iter().any(|s| lower.starts_with(s));
        if !has_question_start {
            return false;
        }
    }

    // Emoji-heavy messages
    let emoji_like = trimmed.chars().filter(|c| {
        let cp = *c as u32;
        (0x1F600..=0x1F64F).contains(&cp)
            || (0x1F300..=0x1F5FF).contains(&cp)
            || (0x1F680..=0x1F6FF).contains(&cp)
            || (0x1F1E0..=0x1F1FF).contains(&cp)
            || (0x2600..=0x26FF).contains(&cp)
            || (0x2700..=0x27BF).contains(&cp)
    }).count();
    if emoji_like > 3 && alphanumeric_count < 5 {
        return false;
    }

    // Contains a question mark AND has actual words
    if trimmed.contains('?') && alphanumeric_count >= 3 {
        return true;
    }

    // Starts with common question words
    let question_starts = [
        "what", "when", "where", "why", "how", "who", "which", "whose", "whom",
        "can you", "could you", "would you", "will you", "did you", "do you",
        "are you", "is it", "should i", "shall we", "may i",
        "has anyone", "does anyone", "did anyone",
        "would it", "will it", "can it", "is there", "are there",
    ];
    for start in &question_starts {
        if lower.starts_with(start) {
            return true;
        }
    }

    // Contains explicit request patterns
    let request_patterns = [
        "let me know",
        "what do you think",
        "your thoughts",
        "please confirm",
        "please reply",
        "please respond",
        "can you confirm",
        "confirm",
        "approve",
        "yes or no",
        "what about",
        "how about",
        "should we",
        "do you want",
        "are you free",
        "you free",
        "free tomorrow",
        "free tonight",
        "free this",
        "want to",
        "wanna",
        "up for",
        "down for",
        "interested in",
    ];
    for pattern in &request_patterns {
        if lower.contains(pattern) {
            return true;
        }
    }

    // Binary choice patterns
    if lower.contains(" or ") && trimmed.len() > 15 {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_false_positives_filtered() {
        assert!(!looks_like_question("240!?"));
        assert!(!looks_like_question("240!!!"));
        assert!(!looks_like_question("lol"));
        assert!(!looks_like_question("haha"));
        assert!(!looks_like_question("ok"));
        assert!(!looks_like_question("nice"));
        assert!(!looks_like_question("wtf"));
        assert!(!looks_like_question("brb"));
        assert!(!looks_like_question("🎉"));
        assert!(!looks_like_question("3-1"));
        assert!(!looks_like_question("$420"));
    }

    #[test]
    fn test_real_questions_detected() {
        assert!(looks_like_question("Are you free?"));
        assert!(looks_like_question("Want to grab lunch?"));
        assert!(looks_like_question("What time works?"));
        assert!(looks_like_question("Can you confirm?"));
        assert!(looks_like_question("Should I deploy?"));
        assert!(looks_like_question("Let me know what you think"));
        assert!(looks_like_question("Yes or no?"));
        assert!(looks_like_question("How about Thai or Indian?"));
        assert!(looks_like_question("What do you think about the proposal?"));
    }

    #[test]
    fn test_edge_cases() {
        assert!(!looks_like_question("how"));
        assert!(looks_like_question("how so?"));
        assert!(looks_like_question("what!?"));
    }
}
