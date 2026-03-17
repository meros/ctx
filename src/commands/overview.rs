use anyhow::Result;
use clap::Args;
use std::fs;
use std::path::PathBuf;

use crate::filter;

#[derive(Args)]
pub struct OverviewArgs {
    /// Root directory to overview
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Maximum depth for the tree
    #[arg(short = 'd', long = "depth", default_value = "3")]
    pub max_depth: usize,

    /// Filter output through Claude with a question
    #[arg(long)]
    pub ask: Option<String>,
}

pub fn run(args: OverviewArgs) -> Result<()> {
    let root = args.path.canonicalize().unwrap_or(args.path.clone());
    let mut output = String::new();

    // 1. Project identity
    output.push_str("# Project Overview\n\n");

    // Detect project type and show key config
    let config_files = [
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "Gemfile",
        "mix.exs",
        "flake.nix",
        "shell.nix",
        "default.nix",
    ];

    for config in &config_files {
        let path = root.join(config);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                let preview = truncate_lines(&content, 30);
                output.push_str(&format!("## {}\n```\n{}\n```\n\n", config, preview));
            }
        }
    }

    // 2. README
    let readme_names = ["README.md", "README", "README.txt", "readme.md"];
    for name in &readme_names {
        let path = root.join(name);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                let preview = truncate_lines(&content, 80);
                output.push_str(&format!("## {}\n{}\n\n", name, preview));
            }
            break;
        }
    }

    // 3. LLM instruction files
    let llm_files = [
        "CLAUDE.md",
        "AGENTS.md",
        "LLMS.md",
        ".cursorrules",
        "copilot-instructions.md",
        ".github/copilot-instructions.md",
    ];
    for name in &llm_files {
        let path = root.join(name);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                output.push_str(&format!("## {} (LLM instructions)\n{}\n\n", name, content));
            }
        }
    }

    // 4. Directory tree (compact)
    output.push_str("## Directory Structure\n```\n");
    let tree_output = build_compact_tree(&root, args.max_depth)?;
    output.push_str(&tree_output);
    output.push_str("```\n\n");

    // 5. Key structural hints
    let workspace_hints = detect_workspaces(&root);
    if !workspace_hints.is_empty() {
        output.push_str("## Workspaces/Packages\n");
        for hint in &workspace_hints {
            output.push_str(&format!("- {}\n", hint));
        }
        output.push('\n');
    }

    let output = filter::maybe_filter(&output, &args.ask)?;
    print!("{}", output);
    Ok(())
}

fn truncate_lines(content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= max_lines {
        content.to_string()
    } else {
        let truncated: String = lines[..max_lines].join("\n");
        format!("{}\n... ({} more lines)", truncated, lines.len() - max_lines)
    }
}

fn build_compact_tree(root: &PathBuf, max_depth: usize) -> Result<String> {
    use ignore::WalkBuilder;

    let walker = WalkBuilder::new(root)
        .max_depth(Some(max_depth))
        .git_ignore(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "node_modules" | ".git" | "dist" | "build" | ".next" | "__pycache__" | "target"
                | ".turbo" | ".cache"
            )
        })
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    let mut output = String::new();
    for entry in walker.flatten() {
        let path = entry.path();
        if path == root.as_path() {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        let depth = rel.components().count();
        let indent = "  ".repeat(depth.saturating_sub(1));
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if path.is_dir() {
            output.push_str(&format!("{}{}/\n", indent, name));
        } else {
            output.push_str(&format!("{}{}\n", indent, name));
        }
    }
    Ok(output)
}

fn detect_workspaces(root: &PathBuf) -> Vec<String> {
    let mut hints = Vec::new();

    // Check package.json for workspaces
    let pkg_path = root.join("package.json");
    if pkg_path.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(workspaces) = json.get("workspaces") {
                    if let Some(arr) = workspaces.as_array() {
                        for ws in arr {
                            if let Some(s) = ws.as_str() {
                                hints.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Check Cargo.toml for workspace members
    let cargo_path = root.join("Cargo.toml");
    if cargo_path.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_path) {
            // Simple parse for [workspace] members
            let mut in_workspace = false;
            for line in content.lines() {
                if line.trim() == "[workspace]" {
                    in_workspace = true;
                } else if line.starts_with('[') && in_workspace {
                    in_workspace = false;
                }
                if in_workspace && line.contains('"') {
                    let member = line.trim().trim_matches(|c: char| c == '"' || c == ',' || c == ' ');
                    if !member.is_empty() && !member.starts_with('[') && !member.starts_with("members") {
                        hints.push(member.to_string());
                    }
                }
            }
        }
    }

    hints
}
