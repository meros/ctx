use anyhow::Result;
use clap::Args;
use ignore::WalkBuilder;
use regex::Regex;
use std::path::PathBuf;

use crate::filter;

#[derive(Args)]
pub struct FindArgs {
    /// Glob pattern to match file names (e.g., '*Model*', '*.graphql')
    pub pattern: String,

    /// Root directory to search in
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Filter by file extension (e.g., ts, rs, py)
    #[arg(short = 't', long = "type")]
    pub file_type: Option<String>,

    /// Maximum number of results
    #[arg(short = 'n', long, default_value = "500")]
    pub limit: usize,

    /// Include gitignored files
    #[arg(long = "no-gitignore")]
    pub no_gitignore: bool,

    /// Filter output through Claude with a question
    #[arg(long)]
    pub ask: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: FindArgs) -> Result<()> {
    let matcher = glob_to_regex(&args.pattern);

    let walker = WalkBuilder::new(&args.path)
        .git_ignore(!args.no_gitignore)
        .git_global(!args.no_gitignore)
        .hidden(false)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "node_modules" | ".git" | "dist" | "build" | ".next" | "__pycache__" | "target"
                    | ".turbo" | ".cache"
            )
        })
        .build();

    let mut results: Vec<String> = Vec::new();

    for entry in walker.flatten() {
        if results.len() >= args.limit {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy(),
            None => continue,
        };

        if !matcher.is_match(&name) {
            continue;
        }

        if let Some(ref ext) = args.file_type {
            if path.extension().and_then(|e| e.to_str()) != Some(ext.as_str()) {
                continue;
            }
        }

        results.push(path.to_string_lossy().to_string());
    }

    results.sort();

    let output = if args.json {
        serde_json::to_string_pretty(&results)?
    } else if results.is_empty() {
        String::new()
    } else {
        results.join("\n") + "\n"
    };

    let output = filter::maybe_filter(&output, &args.ask)?;
    print!("{}", output);
    Ok(())
}

/// Convert a simple glob pattern to a regex.
/// Supports: *, ?, and character classes.
fn glob_to_regex(pattern: &str) -> Regex {
    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' | '+' | '(' | ')' | '{' | '}' | '[' | ']' | '^' | '$' | '|' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    regex.push('$');
    Regex::new(&regex).unwrap_or_else(|_| Regex::new(".*").unwrap())
}
