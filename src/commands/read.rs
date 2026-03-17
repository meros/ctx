use anyhow::{Context, Result};
use clap::Args;
use std::fs;
use std::path::PathBuf;

use crate::filter;
use crate::ts;

use super::CommonArgs;

#[derive(Args)]
pub struct ReadArgs {
    /// Files to read (supports multiple)
    #[arg(required = true)]
    pub files: Vec<PathBuf>,

    /// Line range to read (e.g., "1-50", "100-200")
    #[arg(short = 'l', long = "lines")]
    pub line_range: Option<String>,

    /// Show only lines matching this pattern (post-read filter)
    #[arg(long = "match")]
    pub match_pattern: Option<String>,

    /// Show only signatures/types/imports (tree-sitter skeleton, ~5-50x fewer tokens)
    #[arg(long)]
    pub skeleton: bool,
}

pub fn run(args: ReadArgs, common: &CommonArgs) -> Result<()> {
    let mut output = String::new();
    let (start_line, end_line) = parse_line_range(&args.line_range)?;

    for file in &args.files {
        let content = fs::read_to_string(file)
            .with_context(|| format!("Failed to read: {}", file.display()))?;

        let content = if args.skeleton {
            if let Some(lang) = ts::Lang::from_path(file) {
                ts::extract_skeleton(&content, lang)
            } else {
                content
            }
        } else {
            content
        };

        let lines: Vec<&str> = content.lines().collect();
        let start = start_line.unwrap_or(1).saturating_sub(1);
        let end = end_line.unwrap_or(lines.len()).min(lines.len());

        output.push_str(&format!("=== {} (lines {}-{} of {}) ===\n", file.display(), start + 1, end, lines.len()));

        for (i, line) in lines[start..end].iter().enumerate() {
            let line_num = start + i + 1;

            if let Some(ref pat) = args.match_pattern {
                if !line.contains(pat.as_str()) {
                    continue;
                }
            }

            output.push_str(&format!("{:4} {}\n", line_num, line));
        }

        if args.files.len() > 1 {
            output.push('\n');
        }
    }

    if common.json {
        let json = serde_json::json!({
            "files": args.files.iter().map(|f| f.to_string_lossy().to_string()).collect::<Vec<_>>(),
            "content": output,
        });
        let output = serde_json::to_string_pretty(&json)?;
        let output = if let Some(max_tokens) = common.tokens {
            crate::tokens::truncate_to_tokens(&output, max_tokens)
        } else {
            output
        };
        let output = filter::maybe_filter(&output, &common.ask)?;
        print!("{}", output);
    } else {
        let output = if let Some(max_tokens) = common.tokens {
            crate::tokens::truncate_to_tokens(&output, max_tokens)
        } else {
            output
        };
        let output = filter::maybe_filter(&output, &common.ask)?;
        print!("{}", output);
    }

    Ok(())
}

fn parse_line_range(range: &Option<String>) -> Result<(Option<usize>, Option<usize>)> {
    match range {
        None => Ok((None, None)),
        Some(r) => {
            let parts: Vec<&str> = r.split('-').collect();
            match parts.len() {
                1 => {
                    let n: usize = parts[0].parse().context("Invalid line number")?;
                    Ok((Some(n), Some(n)))
                }
                2 => {
                    let start: usize = parts[0].parse().context("Invalid start line")?;
                    let end: usize = parts[1].parse().context("Invalid end line")?;
                    Ok((Some(start), Some(end)))
                }
                _ => anyhow::bail!("Invalid line range format. Use N or N-M"),
            }
        }
    }
}
