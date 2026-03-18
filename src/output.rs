//! Common output pipeline: truncation → filtering → printing.

use anyhow::Result;
use crate::commands::CommonArgs;
use crate::filter;
use crate::tokens;

/// Apply token truncation, optional LLM filtering, and print.
pub fn emit(output: &str, common: &CommonArgs) -> Result<()> {
    let output = if let Some(max_tokens) = common.tokens {
        tokens::truncate_to_tokens(output, max_tokens)
    } else {
        output.to_string()
    };
    let output = filter::maybe_filter(&output, &common.ask)?;
    print!("{}", output);
    Ok(())
}
