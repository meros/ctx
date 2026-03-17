use anyhow::Result;
use clap::Args;
use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::filter;
use crate::ts;

#[derive(Args)]
pub struct GrepArgs {
    /// Regex pattern to search for
    pub pattern: String,

    /// Directory or file to search in
    #[arg(default_value = ".")]
    pub path: PathBuf,

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

    /// Filter output through Claude with a question
    #[arg(long)]
    pub ask: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: GrepArgs) -> Result<()> {
    let matcher = if args.ignore_case {
        RegexMatcher::new_line_matcher(&format!("(?i){}", args.pattern))?
    } else {
        RegexMatcher::new_line_matcher(&args.pattern)?
    };

    // Collect matching files and line numbers
    let mut file_matches: BTreeMap<PathBuf, Vec<(u64, String)>> = BTreeMap::new();
    let files = collect_search_files(&args)?;

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
            file_matches.insert(file_path.clone(), matches);
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
        format_context(&file_matches, &args, &matcher)?
    };

    let output = filter::maybe_filter(&output, &args.ask)?;
    print!("{}", output);
    Ok(())
}

fn collect_search_files(args: &GrepArgs) -> Result<Vec<PathBuf>> {
    if args.path.is_file() {
        return Ok(vec![args.path.clone()]);
    }

    let mut files = Vec::new();
    let walker = WalkBuilder::new(&args.path)
        .git_ignore(true)
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

        if args.no_tests {
            if is_test_file(&path) {
                continue;
            }
        }

        files.push(path);
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

fn format_files_only(matches: &BTreeMap<PathBuf, Vec<(u64, String)>>) -> String {
    matches
        .keys()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn format_count(matches: &BTreeMap<PathBuf, Vec<(u64, String)>>) -> String {
    matches
        .iter()
        .map(|(p, m)| format!("{}:{}", p.display(), m.len()))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn format_context(
    matches: &BTreeMap<PathBuf, Vec<(u64, String)>>,
    args: &GrepArgs,
    matcher: &RegexMatcher,
) -> Result<String> {
    let mut output = String::new();
    let ctx = args.context_lines;

    for (file_path, file_matches) in matches {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        output.push_str(&format!("=== {} ===\n", file_path.display()));

        for (line_num, _) in file_matches {
            let ln = *line_num as usize;
            let start = ln.saturating_sub(ctx + 1);
            let end = (ln + ctx).min(lines.len());

            for i in start..end {
                let marker = if i + 1 == ln { ">" } else { " " };
                output.push_str(&format!("{}{:4} {}\n", marker, i + 1, lines[i]));
            }
            output.push_str("--\n");
        }
        output.push('\n');
    }

    // Suppress unused variable warning
    let _ = matcher;
    Ok(output)
}

fn format_expanded(
    matches: &BTreeMap<PathBuf, Vec<(u64, String)>>,
    args: &GrepArgs,
) -> Result<String> {
    let mut result = String::new();

    for (file_path, file_matches) in matches {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        // Parse with tree-sitter if using --fn
        let tree = if args.expand_fn {
            ts::Lang::from_path(file_path).and_then(|lang| ts::parse(&content, lang).ok())
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
                    ts::find_enclosing_function(tree, &content, match_line - 1, lang)
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
