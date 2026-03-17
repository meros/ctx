/// Estimate token count for text (chars/4 approximation).
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Truncate text to fit within a token budget.
/// Returns the text as-is if it fits, otherwise truncates at a line boundary with a notice.
pub fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    let estimated = estimate_tokens(text);
    if estimated <= max_tokens {
        return text.to_string();
    }

    // Target byte count (4 bytes per token approximation)
    let target_bytes = max_tokens * 4;

    // Find a clean line boundary near the target
    let truncated = &text[..target_bytes.min(text.len())];
    let last_newline = truncated.rfind('\n').unwrap_or(truncated.len());
    let clean_cut = &text[..last_newline];

    let remaining_lines = text[last_newline..].lines().count();
    let remaining_tokens = estimate_tokens(&text[last_newline..]);

    format!(
        "{}\n\n[... truncated at ~{} tokens — {} more lines, ~{} more tokens ...]\n",
        clean_cut, max_tokens, remaining_lines, remaining_tokens
    )
}
