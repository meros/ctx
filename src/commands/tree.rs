use anyhow::Result;
use clap::Args;
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::filter;

#[derive(Args)]
pub struct TreeArgs {
    /// Root directory to show tree for (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Maximum depth to traverse
    #[arg(short = 'd', long = "depth", default_value = "4")]
    pub max_depth: usize,

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

pub fn run(args: TreeArgs) -> Result<()> {
    let root = args.path.canonicalize().unwrap_or(args.path.clone());
    let mut entries: Vec<PathBuf> = Vec::new();

    let walker = WalkBuilder::new(&root)
        .max_depth(Some(args.max_depth))
        .git_ignore(!args.no_gitignore)
        .git_global(!args.no_gitignore)
        .hidden(false) // show dotfiles like .env.example
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            // Skip common noise directories even if not gitignored
            !matches!(
                name.as_ref(),
                "node_modules" | ".git" | "dist" | "build" | ".next" | "__pycache__" | "target"
                | ".turbo" | ".cache" | ".parcel-cache"
            )
        })
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    for entry in walker.flatten() {
        let path = entry.path().to_path_buf();
        if path != root {
            entries.push(path);
        }
    }

    if args.json {
        let json_entries: Vec<serde_json::Value> = entries
            .iter()
            .map(|p| {
                let rel = p.strip_prefix(&root).unwrap_or(p);
                serde_json::json!({
                    "path": rel.to_string_lossy(),
                    "is_dir": p.is_dir(),
                })
            })
            .collect();
        let output = serde_json::to_string_pretty(&json_entries)?;
        let output = filter::maybe_filter(&output, &args.ask)?;
        print!("{}", output);
        return Ok(());
    }

    let output = format_tree(&root, &entries);
    let output = filter::maybe_filter(&output, &args.ask)?;
    print!("{}", output);
    Ok(())
}

fn format_tree(root: &Path, entries: &[PathBuf]) -> String {
    // Build a tree structure using BTreeMap for sorted output
    let mut tree: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();

    for entry in entries {
        let rel = entry.strip_prefix(root).unwrap_or(entry);
        if let Some(parent) = rel.parent() {
            tree.entry(parent.to_path_buf())
                .or_default()
                .push(rel.to_path_buf());
        }
    }

    let root_name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let mut output = format!("{}/\n", root_name);
    let root_path = PathBuf::new();
    if let Some(children) = tree.get(&root_path) {
        let mut sorted = children.clone();
        sorted.sort();
        sorted.dedup();
        format_children(&mut output, &tree, &sorted, "", entries);
    }
    output
}

fn format_children(
    output: &mut String,
    tree: &BTreeMap<PathBuf, Vec<PathBuf>>,
    children: &[PathBuf],
    prefix: &str,
    all_entries: &[PathBuf],
) {
    for (i, child) in children.iter().enumerate() {
        let is_last = i == children.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let name = child
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let is_dir = tree.contains_key(child)
            || all_entries.iter().any(|e| {
                let rel = e.strip_prefix(e.ancestors().last().unwrap_or(e)).unwrap_or(e);
                rel.starts_with(child) && rel != child
            });

        if is_dir {
            output.push_str(&format!("{}{}{}/ \n", prefix, connector, name));
        } else {
            output.push_str(&format!("{}{}{}\n", prefix, connector, name));
        }

        // Recurse into directory children
        if let Some(sub_children) = tree.get(child) {
            let new_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
            let mut sorted = sub_children.clone();
            sorted.sort();
            sorted.dedup();
            format_children(output, tree, &sorted, &new_prefix, all_entries);
        }
    }
}
