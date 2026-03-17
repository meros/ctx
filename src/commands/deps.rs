use anyhow::{Context, Result};
use clap::Args;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::filter;
use crate::ts;

#[derive(Args)]
pub struct DepsArgs {
    /// File to trace dependencies for
    pub file: PathBuf,

    /// Maximum depth of dependency traversal
    #[arg(short = 'd', long = "depth", default_value = "3")]
    pub max_depth: usize,

    /// Only show external (package) imports
    #[arg(long = "external")]
    pub external_only: bool,

    /// Only show local (relative) imports
    #[arg(long = "local")]
    pub local_only: bool,

    /// Filter output through Claude with a question
    #[arg(long)]
    pub ask: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Maximum output size in estimated tokens (truncates with notice)
    #[arg(long)]
    pub tokens: Option<usize>,
}

pub fn run(args: DepsArgs) -> Result<()> {
    let file = args.file.canonicalize()
        .with_context(|| format!("File not found: {}", args.file.display()))?;

    let mut output = String::new();
    let mut visited = HashSet::new();

    output.push_str(&format!("Dependencies for: {}\n\n", args.file.display()));
    trace_deps(&file, 0, args.max_depth, &mut visited, &mut output, &args)?;

    let output = if let Some(max_tokens) = args.tokens {
        crate::tokens::truncate_to_tokens(&output, max_tokens)
    } else {
        output
    };
    let output = filter::maybe_filter(&output, &args.ask)?;
    print!("{}", output);
    Ok(())
}

fn trace_deps(
    file: &Path,
    depth: usize,
    max_depth: usize,
    visited: &mut HashSet<PathBuf>,
    output: &mut String,
    args: &DepsArgs,
) -> Result<()> {
    if depth > max_depth || visited.contains(file) {
        return Ok(());
    }
    visited.insert(file.to_path_buf());

    let content = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let lang = match ts::Lang::from_path(file) {
        Some(l) => l,
        None => return Ok(()), // Unsupported language
    };

    let tree = ts::parse(&content, lang)?;
    let imports = ts::extract_imports(&tree, &content, lang);
    let indent = "  ".repeat(depth);

    // Filter imports based on args
    let filtered: Vec<_> = imports.iter().filter(|import| {
        let is_local = import.path.starts_with('.') || import.path.starts_with('/');
        if args.external_only && is_local {
            return false;
        }
        if args.local_only && !is_local {
            return false;
        }
        true
    }).collect();

    for (i, import) in filtered.iter().enumerate() {
        let is_last = i == filtered.len() - 1;
        let connector = if is_last { "└──" } else { "├──" };
        let is_local = import.path.starts_with('.') || import.path.starts_with('/');

        output.push_str(&format!("{}{} {} ({}, line {})\n", indent, connector, import.path, import.kind, import.line + 1));

        // Recursively trace local imports
        if is_local && depth < max_depth {
            if let Some(resolved) = resolve_import(&import.path, file) {
                trace_deps(&resolved, depth + 1, max_depth, visited, output, args)?;
            }
        }
    }

    Ok(())
}

fn resolve_import(import_path: &str, from_file: &Path) -> Option<PathBuf> {
    let dir = from_file.parent()?;
    let candidate = dir.join(import_path);

    // Try exact path
    if candidate.exists() && candidate.is_file() {
        return Some(candidate);
    }

    // Try common extensions
    let extensions = ["ts", "tsx", "js", "jsx", "rs", "py"];
    for ext in &extensions {
        let with_ext = candidate.with_extension(ext);
        if with_ext.exists() {
            return Some(with_ext);
        }
    }

    // Try index file in directory
    for ext in &extensions {
        let index = candidate.join(format!("index.{}", ext));
        if index.exists() {
            return Some(index);
        }
    }

    None
}
