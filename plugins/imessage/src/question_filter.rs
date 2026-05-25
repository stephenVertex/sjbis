/// Heuristic: does this message look like it requires a human response?
///
/// Revised logic (2026-05-24): checks for non-questions FIRST, then looks for
/// actual question signals. A lone ? on a short reaction is not a question.
///
/// Order matters:
/// 1. Hard no: very short reactions, emojis, known non-question patterns
/// 2. Strong yes: question words, request patterns, explicit choice language
/// 3. Ambiguous: ? alone — only counts if message has substance
pub fn looks_like_question(text: &str) -> bool {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();

    // ── 1. HARD NO — obvious non-questions ────────────────────────────────

    // Very short (just reactions, emojis, exclamations)
    if trimmed.len() < 4 {
        return false;
    }

    // Exact-match non-questions: standalone reactions/confirmations.
    // These ONLY match when the entire message is the word (no starts_with).
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

    // Starts-with non-questions: longer phrases that are clearly not questions
    // even when followed by more text. These are safe because they're multi-word.
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

    // Messages that are mostly punctuation / symbols / numbers (reactions, scores, times)
    // e.g. "240!?", "lol!!!", "12:30", "score: 3-1", "$420"
    let alphanumeric_count = trimmed.chars().filter(|c| c.is_alphabetic()).count();
    if alphanumeric_count < 3 {
        // Very few letters — probably not a real question
        // Unless it explicitly starts with a question word
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

    // Emoji-only or mostly-emoji messages
    let emoji_like = trimmed.chars().filter(|c| {
        let cp = *c as u32;
        // Common emoji ranges
        (0x1F600..=0x1F64F).contains(&cp) ||  // emoticons
        (0x1F300..=0x1F5FF).contains(&cp) ||  // symbols & pictographs
        (0x1F680..=0x1F6FF).contains(&cp) ||  // transport & map
        (0x1F1E0..=0x1F1FF).contains(&cp) ||  // flags
        (0x2600..=0x26FF).contains(&cp) ||    // misc symbols
        (0x2700..=0x27BF).contains(&cp)       // dingbats
    }).count();
    if emoji_like > 3 && alphanumeric_count < 5 {
        return false;
    }

    // ── 2. STRONG YES — clear question signals ─────────────────────────────

    // Contains a question mark AND has actual words (not just symbols)
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

    // Binary choice patterns (A or B, pick one)
    if lower.contains(" or ") && trimmed.len() > 15 {
        return true;
    }

    false
}

/// Try to extract multiple-choice options from the text
pub fn infer_choices(text: &str) -> Vec<String> {
    let mut choices = Vec::new();

    // Look for patterns like "A or B" or "A, B, or C"
    let or_patterns: Vec<&str> = text.split(" or ").collect();
    if or_patterns.len() >= 2 {
        for part in &or_patterns {
            let trimmed = part.trim().trim_end_matches(',').trim_end_matches('?');
            if !trimmed.is_empty() && trimmed.len() < 30 {
                choices.push(trimmed.to_string());
            }
        }
    }

    // Look for comma-separated short options
    if choices.is_empty() {
        let comma_parts: Vec<&str> = text.split(", ").collect();
        if comma_parts.len() >= 2 && comma_parts.len() <= 5 {
            let all_short = comma_parts.iter().all(|p| p.len() < 20);
            if all_short {
                for part in &comma_parts {
                    let trimmed = part.trim().trim_end_matches(" or").trim_end_matches("?");
                    if !trimmed.is_empty() {
                        choices.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    // Look for numbered options: 1) foo 2) bar
    if choices.is_empty() {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() >= 2 {
            let mut numbered = Vec::new();
            for line in &lines {
                let trimmed = line.trim();
                if trimmed.starts_with(|c: char| c.is_ascii_digit())
                    && (trimmed.contains(')') || trimmed.contains('.')) {
                    let without_prefix = trimmed
                        .trim_start_matches(|c: char| c.is_ascii_digit())
                        .trim_start_matches(|c| c == ')' || c == '.')
                        .trim();
                    if !without_prefix.is_empty() && without_prefix.len() < 40 {
                        numbered.push(without_prefix.to_string());
                    }
                }
            }
            if numbered.len() >= 2 {
                choices = numbered;
            }
        }
    }

    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_false_positives_filtered() {
        // These should NOT surface
        assert!(!looks_like_question("240!?"));
        assert!(!looks_like_question("240!!!"));
        assert!(!looks_like_question("lol"));
        assert!(!looks_like_question("haha"));
        assert!(!looks_like_question("ok"));
        assert!(!looks_like_question("nice"));
        assert!(!looks_like_question("wtf"));
        assert!(!looks_like_question("brb"));
        assert!(!looks_like_question("😂"));
        assert!(!looks_like_question("🎉"));
        assert!(!looks_like_question("👍"));
        assert!(!looks_like_question("3-1"));
        assert!(!looks_like_question("$420"));
    }

    #[test]
    fn test_real_questions_detected() {
        // These SHOULD surface
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
        // Very short but actual question words
        assert!(!looks_like_question("how"));           // too short
        assert!(looks_like_question("how so?"));       // has substance + ?
        assert!(looks_like_question("what!?"));          // question word + ?
    }
}
