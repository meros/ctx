use anyhow::{Context, Result};
use clap::Args;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;

use crate::ts::{self, RefKind};
use crate::walker;

use super::CommonArgs;

#[derive(Args)]
pub struct FlowArgs {
    /// Symbol to trace through the codebase
    pub symbol: String,

    /// Paths to search (default: current directory)
    #[arg(default_value = ".")]
    pub paths: Vec<PathBuf>,

    /// Filter by file type extension (e.g., ts, rs, py)
    #[arg(short = 't', long = "type")]
    pub file_type: Option<String>,

    /// Exclude test files
    #[arg(long = "no-tests")]
    pub no_tests: bool,

    /// Maximum number of files to search
    #[arg(short = 'n', long = "limit", default_value = "50")]
    pub limit: usize,
}

struct FlowSite {
    file: PathBuf,
    line: usize, // 0-indexed
    kind: RefKind,
    snippet: String,
}

pub fn run(args: FlowArgs, common: &CommonArgs) -> Result<()> {
    // Phase 1: grep for word-bounded symbol
    let pattern = format!(r"\b{}\b", regex_escape_ident(&args.symbol));
    let matcher = RegexMatcher::new_line_matcher(&pattern)
        .with_context(|| format!("Invalid symbol name: {}", args.symbol))?;

    let files = collect_files(&args, common)?;

    let mut file_lines: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    let mut searcher = Searcher::new();
    let mut matched_files = 0;

    for file_path in &files {
        if matched_files >= args.limit {
            break;
        }
        let mut lines: Vec<usize> = Vec::new();
        let _ = searcher.search_path(
            &matcher,
            file_path,
            UTF8(|line_num, _| {
                lines.push(line_num as usize - 1); // 0-indexed
                Ok(true)
            }),
        );
        if !lines.is_empty() {
            file_lines.insert(file_path.clone(), lines);
            matched_files += 1;
        }
    }

    if file_lines.is_empty() {
        println!("No references to '{}' found.", args.symbol);
        return Ok(());
    }

    // Phase 2: parse each file and classify references
    let mut sites: Vec<FlowSite> = Vec::new();
    let mut parser = tree_sitter::Parser::new();

    for (file_path, match_lines) in &file_lines {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let source_lines: Vec<&str> = content.lines().collect();
        let lang = ts::Lang::from_path(file_path);

        // Try AST-based classification if tree-sitter supports this language
        let tree = lang.and_then(|l| ts::parse_with(&mut parser, &content, l).ok());

        for &line in match_lines {
            if let (Some(ref tree), Some(lang)) = (&tree, lang) {
                // AST-aware classification
                let idents = ts::find_identifiers_on_line(tree, &content, line, &args.symbol);

                if idents.is_empty() {
                    // Symbol is in a comment or string — skip
                    continue;
                }

                let mut seen_kinds: HashSet<RefKind> = HashSet::new();
                for ident_node in &idents {
                    let kind = ts::classify_identifier(*ident_node, &content, lang);
                    if seen_kinds.insert(kind) {
                        let snippet =
                            extract_snippet(&source_lines, line, kind, tree, &content, lang);
                        sites.push(FlowSite {
                            file: file_path.clone(),
                            line,
                            kind,
                            snippet,
                        });
                    }
                }
            } else {
                // No tree-sitter support — emit as READ with raw grep snippet
                let snippet = format_raw_snippet(&source_lines, line);
                sites.push(FlowSite {
                    file: file_path.clone(),
                    line,
                    kind: RefKind::Read,
                    snippet,
                });
            }
        }
    }

    if sites.is_empty() {
        println!(
            "Found '{}' in {} files but could not classify references (comments/strings only?).",
            args.symbol,
            file_lines.len()
        );
        return Ok(());
    }

    // Phase 3: sort by kind (narrative order), then file, then line
    sites.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });

    // Phase 4: render
    let output = if common.json {
        render_json(&args.symbol, &sites)?
    } else {
        render_text(&args.symbol, &sites)
    };

    crate::output::emit(&output, common)?;
    Ok(())
}

fn extract_snippet(
    lines: &[&str],
    target: usize,
    kind: RefKind,
    tree: &tree_sitter::Tree,
    source: &str,
    lang: ts::Lang,
) -> String {
    let total = lines.len();

    let (start, end) = match kind {
        // Definitions: show enclosing function/block for context
        RefKind::Definition | RefKind::TypeDefinition => ts::find_enclosing_function(
            tree, source, target, lang,
        )
        .map(|(s, e)| (s, e.min(s + 14))) // cap at 15 lines
        .unwrap_or_else(|| ctx_range(target, 4, total)),
        RefKind::PropPass => ctx_range(target, 4, total),
        RefKind::PropReceive => ctx_range(target, 3, total),
        _ => ctx_range(target, 2, total),
    };

    let mut snippet = String::new();
    for i in start..=end.min(total.saturating_sub(1)) {
        let marker = if i == target { ">" } else { " " };
        snippet.push_str(&format!("{}{:4} {}\n", marker, i + 1, lines[i]));
    }
    snippet
}

fn ctx_range(line: usize, ctx: usize, total: usize) -> (usize, usize) {
    (
        line.saturating_sub(ctx),
        (line + ctx).min(total.saturating_sub(1)),
    )
}

fn render_text(symbol: &str, sites: &[FlowSite]) -> String {
    let mut out = String::new();
    let file_count = sites
        .iter()
        .map(|s| &s.file)
        .collect::<HashSet<_>>()
        .len();

    out.push_str(&format!("Flow: {}\n", symbol));

    let mut current_kind: Option<RefKind> = None;

    for site in sites {
        if current_kind != Some(site.kind) {
            out.push('\n');
            let label = site.kind.label();
            let dashes = "─".repeat(66usize.saturating_sub(label.len() + 3));
            out.push_str(&format!("── {} {}\n", label, dashes));
            current_kind = Some(site.kind);
        }

        out.push_str(&format!("  {}:{}\n", site.file.display(), site.line + 1));
        for line in site.snippet.lines() {
            out.push_str(&format!("    {}\n", line));
        }
    }

    out.push_str(&format!(
        "\n{} sites across {} files\n",
        sites.len(),
        file_count,
    ));

    out
}

fn render_json(symbol: &str, sites: &[FlowSite]) -> Result<String> {
    let json: Vec<serde_json::Value> = sites
        .iter()
        .map(|s| {
            serde_json::json!({
                "symbol": symbol,
                "file": s.file.to_string_lossy(),
                "line": s.line + 1,
                "kind": s.kind.label(),
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&json)?)
}

/// Format a raw snippet for files without tree-sitter support.
fn format_raw_snippet(lines: &[&str], target: usize) -> String {
    let total = lines.len();
    let (start, end) = ctx_range(target, 2, total);
    let mut snippet = String::new();
    for i in start..=end.min(total.saturating_sub(1)) {
        let marker = if i == target { ">" } else { " " };
        snippet.push_str(&format!("{}{:4} {}\n", marker, i + 1, lines[i]));
    }
    snippet
}

fn collect_files(args: &FlowArgs, common: &CommonArgs) -> Result<Vec<PathBuf>> {
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

            if let Some(ref ft) = args.file_type {
                if path.extension().and_then(|e| e.to_str()) != Some(ft.as_str()) {
                    continue;
                }
            }

            if args.no_tests && walker::is_test_file(&path) {
                continue;
            }

            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

/// Escape regex-special characters in a symbol name.
fn regex_escape_ident(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if r"\.+*?()[]{}|^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
