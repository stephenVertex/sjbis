/// Heuristic: does this message look like it requires a human response?
pub fn looks_like_question(text: &str) -> bool {
    let lower = text.to_lowercase();
    
    // Contains a question mark
    if lower.contains('?') {
        return true;
    }
    
    // Starts with common question words
    let question_starts = [
        "what", "when", "where", "why", "how", "who", "which", "whose", "whom",
        "can you", "could you", "would you", "will you", "did you", "do you",
        "are you", "is it", "should i", "shall we", "may i",
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
        "confirm",
        "approve",
        "yes or no",
    ];
    for pattern in &request_patterns {
        if lower.contains(pattern) {
            return true;
        }
    }
    
    // Negative signals: very short, just reactions
    let trimmed = text.trim();
    if trimmed.len() < 5 {
        return false;
    }
    
    // Common non-question patterns
    let non_questions = [
        "lol", "haha", "ok", "okay", "k", "thanks", "ty", "np", 
        "sure", "nice", "cool", "wow", "omg", "brb", "gtg",
        "on my way", "omw", "be there", "see you", "sounds good",
        "got it", "will do", "done", "ok great", "perfect",
    ];
    for pattern in &non_questions {
        if lower == *pattern || lower.starts_with(&format!("{pattern} ")) {
            return false;
        }
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
            let trimmed = part.trim().trim_end_matches(',');
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
        // Check for simple numbered lists
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
