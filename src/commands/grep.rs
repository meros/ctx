use anyhow::Result;
use clap::Args;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::filter;
use crate::ts;
use crate::walker;

use super::CommonArgs;

#[derive(Args)]
pub struct GrepArgs {
    /// Regex pattern to search for (Rust/ripgrep syntax: use | for alternation, () for grouping)
    pub pattern: String,

    /// Paths to search (use -- before paths: ctx grep 'pat' -- src/ lib/)
    #[arg(default_value = ".")]
    pub paths: Vec<PathBuf>,

    /// Filter by file type extension (e.g., ts, rs, py)
    #[arg(short = 't', long = "type")]
    pub file_type: Option<String>,

    /// Lines of context around each match
    #[arg(short = 'C', long = "context", default_value = "2")]
    pub context_lines: usize,

    /// Expand each match to show N lines around it (e.g., 80 to see a full function).
    /// Overrides --context. Shows file:line headers for each block.
    #[arg(short = 'e', long = "expand")]
    pub expand: Option<usize>,

    /// Expand each match to show the full enclosing function/method (AST-aware via tree-sitter).
    /// Best for: "show me this method".
    #[arg(long = "fn")]
    pub expand_fn: bool,

    /// Only show file paths (not content)
    #[arg(short = 'l', long = "files-only")]
    pub files_only: bool,

    /// Show match counts per file
    #[arg(short = 'c', long = "count")]
    pub count: bool,

    /// Maximum number of matching files
    #[arg(short = 'n', long, default_value = "50")]
    pub limit: usize,

    /// Case-insensitive search
    #[arg(short = 'i', long = "ignore-case")]
    pub ignore_case: bool,

    /// Exclude test files (*.test.*, *.spec.*, *_test.*, test_*.*)
    #[arg(long = "no-tests")]
    pub no_tests: bool,
}

/// Detect and convert common BRE (GNU grep basic regex) patterns to Rust/ERE regex.
/// Returns (normalized_pattern, was_modified).
fn normalize_pattern(pattern: &str) -> (String, bool) {
    // Check for BRE-style escaped operators: \|, \(, \), \+, \?
    // These are metacharacters in BRE but literal escapes in Rust regex.
    // We need to be careful not to convert \\| (escaped backslash + pipe).
    let mut result = String::with_capacity(pattern.len());
    let mut modified = false;
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            // Check if this is a double backslash (\\) — skip both
            if chars[i + 1] == '\\' {
                result.push('\\');
                result.push('\\');
                i += 2;
                continue;
            }
            // BRE metacharacters that should be unescaped for Rust regex
            match chars[i + 1] {
                '|' | '(' | ')' | '+' | '?' | '{' | '}' => {
                    result.push(chars[i + 1]);
                    modified = true;
                    i += 2;
                }
                _ => {
                    result.push('\\');
                    result.push(chars[i + 1]);
                    i += 2;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    (result, modified)
}

pub fn run(args: GrepArgs, common: &CommonArgs) -> Result<()> {
    // Normalize BRE patterns to Rust regex
    let (pattern, was_normalized) = normalize_pattern(&args.pattern);
    if was_normalized {
        eprintln!(
            "ctx: auto-converted BRE pattern '{}' → '{}' (ctx uses Rust/ripgrep regex syntax)",
            args.pattern, pattern
        );
    }

    let matcher = match if args.ignore_case {
        RegexMatcher::new_line_matcher(&format!("(?i){}", pattern))
    } else {
        RegexMatcher::new_line_matcher(&pattern)
    } {
        Ok(m) => m,
        Err(e) => {
            anyhow::bail!(
                "Invalid regex pattern '{}': {}\n\n\
                 ctx uses Rust/ripgrep regex syntax:\n\
                 - Alternation: foo|bar  (NOT foo\\|bar)\n\
                 - Grouping:    (a|b)    (NOT \\(a\\|b\\))\n\
                 - Quantifiers: a+  a?   (NOT a\\+ a\\?)\n\
                 - Word boundary: \\bword\\b\n\
                 - Case-insensitive: (?i)pattern or -i flag\n\
                 See: https://docs.rs/regex/latest/regex/#syntax",
                args.pattern, e
            );
        }
    };

    // Collect matching files and line numbers
    // Cache file content alongside matches to avoid double-reads in format functions
    let mut file_matches: FileMatches = BTreeMap::new();
    let files = collect_search_files(&args, common)?;
    let needs_content = !args.files_only && !args.count;

    let mut searcher = Searcher::new();
    for file_path in &files {
        if file_matches.len() >= args.limit {
            break;
        }
        let mut matches: Vec<(u64, String)> = Vec::new();
        let _ = searcher.search_path(
            &matcher,
            file_path,
            UTF8(|line_num, line| {
                matches.push((line_num, line.to_string()));
                Ok(true)
            }),
        );
        if !matches.is_empty() {
            let content = if needs_content {
                fs::read_to_string(file_path).ok()
            } else {
                None
            };
            file_matches.insert(file_path.clone(), (matches, content));
        }
    }

    if file_matches.is_empty() {
        println!("No matches found.");
        return Ok(());
    }

    // Format output based on mode
    let output = if args.files_only {
        format_files_only(&file_matches)
    } else if args.count {
        format_count(&file_matches)
    } else if args.expand_fn || args.expand.is_some() {
        format_expanded(&file_matches, &args)?
    } else {
        format_context(&file_matches, &args)?
    };

    let output = if let Some(max_tokens) = common.tokens {
        crate::tokens::truncate_to_tokens(&output, max_tokens)
    } else {
        output
    };
    let output = filter::maybe_filter(&output, &common.ask)?;
    print!("{}", output);
    Ok(())
}

fn collect_search_files(args: &GrepArgs, common: &CommonArgs) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for root in &args.paths {
        if root.is_file() {
            files.push(root.clone());
            continue;
        }

        let walker = walker::build_walker(root, !common.no_gitignore).build();

        for entry in walker.flatten() {
            let path = entry.path().to_path_buf();
            if !path.is_file() {
                continue;
            }

            if let Some(ref ext) = args.file_type {
                if path.extension().and_then(|e| e.to_str()) != Some(ext.as_str()) {
                    continue;
                }
            }

            if args.no_tests && is_test_file(&path) {
                continue;
            }

            files.push(path);
        }
    }

    Ok(files)
}

fn is_test_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.starts_with("test_")
        || name.contains(".mocha.")
}

type FileMatches = BTreeMap<PathBuf, (Vec<(u64, String)>, Option<String>)>;

fn format_files_only(matches: &FileMatches) -> String {
    matches
        .keys()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn format_count(matches: &FileMatches) -> String {
    matches
        .iter()
        .map(|(p, (m, _))| format!("{}:{}", p.display(), m.len()))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn format_context(
    matches: &FileMatches,
    args: &GrepArgs,
) -> Result<String> {
    let mut output = String::new();
    let ctx = args.context_lines;

    for (file_path, (file_matches, cached_content)) in matches {
        let content = match cached_content {
            Some(c) => c.as_str(),
            None => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        output.push_str(&format!("=== {} ===\n", file_path.display()));

        for (line_num, _) in file_matches {
            let ln = *line_num as usize;
            let start = ln.saturating_sub(ctx + 1);
            let end = (ln + ctx).min(lines.len());

            for (i, line) in lines[start..end].iter().enumerate() {
                let line_idx = start + i + 1;
                let marker = if line_idx == ln { ">" } else { " " };
                output.push_str(&format!("{}{:4} {}\n", marker, line_idx, line));
            }
            output.push_str("--\n");
        }
        output.push('\n');
    }

    Ok(output)
}

fn format_expanded(
    matches: &FileMatches,
    args: &GrepArgs,
) -> Result<String> {
    let mut result = String::new();
    let mut parser = tree_sitter::Parser::new();

    for (file_path, (file_matches, cached_content)) in matches {
        let content = match cached_content {
            Some(c) => c.as_str(),
            None => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        // Parse with tree-sitter if using --fn (reuse parser across files)
        let tree = if args.expand_fn {
            ts::Lang::from_path(file_path).and_then(|lang| ts::parse_with(&mut parser, content, lang).ok())
        } else {
            None
        };
        let lang = ts::Lang::from_path(file_path);

        // Collect all ranges, then deduplicate overlapping ones
        let mut ranges: Vec<(usize, usize, usize)> = Vec::new(); // (start, end, match_line)

        for (line_num, _) in file_matches {
            let match_line = *line_num as usize;

            if args.expand_fn {
                let (start, end) = if let (Some(ref tree), Some(lang)) = (&tree, lang) {
                    ts::find_enclosing_function(tree, content, match_line - 1, lang)
                        .unwrap_or_else(|| {
                            let half = 20;
                            (
                                match_line.saturating_sub(half + 1),
                                (match_line + half - 1).min(lines.len() - 1),
                            )
                        })
                } else {
                    let half = 20;
                    (
                        match_line.saturating_sub(half + 1),
                        (match_line + half - 1).min(lines.len() - 1),
                    )
                };
                ranges.push((start, end, match_line));
            } else if let Some(expand) = args.expand {
                let half = expand / 2;
                let start = match_line.saturating_sub(half + 1);
                let end = (match_line + half - 1).min(lines.len() - 1);
                ranges.push((start, end, match_line));
            }
        }

        // Deduplicate: merge overlapping ranges
        let merged = merge_ranges(&mut ranges);

        for (start, end, match_lines) in &merged {
            result.push_str(&format!(
                "=== {}:{}-{} ===\n",
                file_path.display(),
                start + 1,
                end + 1
            ));
            for (i, line) in lines[*start..=(*end).min(lines.len() - 1)]
                .iter()
                .enumerate()
            {
                let ln = start + i + 1;
                let marker = if match_lines.contains(&ln) { ">" } else { " " };
                result.push_str(&format!("{}{:4} {}\n", marker, ln, line));
            }
            result.push('\n');
        }
    }

    Ok(result)
}

/// Merge overlapping (start, end, match_line) ranges into deduplicated blocks.
fn merge_ranges(ranges: &mut [(usize, usize, usize)]) -> Vec<(usize, usize, Vec<usize>)> {
    if ranges.is_empty() {
        return vec![];
    }
    ranges.sort_by_key(|r| (r.0, r.1));

    let mut merged: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    let (mut cur_start, mut cur_end, first_match) = ranges[0];
    let mut cur_matches = vec![first_match];

    for &(start, end, match_line) in &ranges[1..] {
        if start <= cur_end + 1 {
            // Overlapping or adjacent — merge
            cur_end = cur_end.max(end);
            cur_matches.push(match_line);
        } else {
            merged.push((cur_start, cur_end, cur_matches));
            cur_start = start;
            cur_end = end;
            cur_matches = vec![match_line];
        }
    }
    merged.push((cur_start, cur_end, cur_matches));

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_bre_alternation() {
        let (result, modified) = normalize_pattern(r"foo\|bar\|baz");
        assert_eq!(result, "foo|bar|baz");
        assert!(modified);
    }

    #[test]
    fn test_normalize_bre_groups() {
        let (result, modified) = normalize_pattern(r"\(foo\|bar\)");
        assert_eq!(result, "(foo|bar)");
        assert!(modified);
    }

    #[test]
    fn test_normalize_bre_quantifiers() {
        let (result, modified) = normalize_pattern(r"foo\+bar\?");
        assert_eq!(result, "foo+bar?");
        assert!(modified);
    }

    #[test]
    fn test_normalize_preserves_valid_escapes() {
        let (result, modified) = normalize_pattern(r"\bword\b");
        assert_eq!(result, r"\bword\b");
        assert!(!modified);
    }

    #[test]
    fn test_normalize_preserves_double_backslash() {
        let (result, modified) = normalize_pattern(r"foo\\|bar");
        assert_eq!(result, r"foo\\|bar");
        assert!(!modified);
    }

    #[test]
    fn test_normalize_plain_alternation_unchanged() {
        let (result, modified) = normalize_pattern("foo|bar");
        assert_eq!(result, "foo|bar");
        assert!(!modified);
    }

    #[test]
    fn test_normalize_no_change() {
        let (result, modified) = normalize_pattern("simple");
        assert_eq!(result, "simple");
        assert!(!modified);
    }
}
