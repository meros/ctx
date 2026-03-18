use anyhow::{Context, Result};
use clap::Args;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::resolve;
use crate::ts;
use crate::walker;

use super::CommonArgs;

#[derive(Args)]
pub struct ChainArgs {
    /// Target file to trace
    pub file: PathBuf,

    /// Second file — find import path from <file> to <to>
    pub to: Option<PathBuf>,

    /// Forward traversal (what does this file import?) instead of reverse
    #[arg(long)]
    pub forward: bool,

    /// Maximum traversal depth
    #[arg(short = 'd', long = "depth", default_value = "5")]
    pub max_depth: usize,

    /// Filter source files by extension (e.g., ts, rs, py)
    #[arg(short = 't', long = "type")]
    pub file_type: Option<String>,

    /// Exclude test files
    #[arg(long = "no-tests")]
    pub no_tests: bool,

    /// Show skeleton (signatures only) of found files
    #[arg(long)]
    pub skeleton: bool,
}

struct ImportGraph {
    forward: HashMap<PathBuf, Vec<PathBuf>>,
    reverse: HashMap<PathBuf, Vec<PathBuf>>,
}

pub fn run(args: ChainArgs, common: &CommonArgs) -> Result<()> {
    let file = args.file.canonicalize()
        .with_context(|| format!("File not found: {}", args.file.display()))?;

    let to = if let Some(ref to_path) = args.to {
        Some(to_path.canonicalize()
            .with_context(|| format!("File not found: {}", to_path.display()))?)
    } else {
        None
    };

    // Determine project root (walk up to find .git or use cwd)
    let root = find_project_root(&file).unwrap_or_else(|| std::env::current_dir().unwrap());

    let graph = build_import_graph(&root, &args, common)?;

    let output = if let Some(ref to_file) = to {
        // Path mode: find chain from file → to
        render_path(&file, to_file, &graph, &args, common)?
    } else if args.forward {
        // Forward mode: what does this file import?
        render_traversal(&file, &graph.forward, "forward", &args, common)?
    } else {
        // Reverse mode (default): who imports this file?
        render_traversal(&file, &graph.reverse, "reverse", &args, common)?
    };

    crate::output::emit(&output, common)
}

fn find_project_root(from: &Path) -> Option<PathBuf> {
    let mut dir = from.parent()?;
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

fn build_import_graph(root: &Path, args: &ChainArgs, common: &CommonArgs) -> Result<ImportGraph> {
    let mut forward: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    let mut reverse: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    let mut parser = tree_sitter::Parser::new();

    let walker = walker::build_walker(root, !common.no_gitignore).build();

    for entry in walker.flatten() {
        let path = entry.path().to_path_buf();
        if !path.is_file() {
            continue;
        }

        // Filter by type if specified
        if let Some(ref ft) = args.file_type {
            if path.extension().and_then(|e| e.to_str()) != Some(ft.as_str()) {
                continue;
            }
        }

        // Skip test files if requested
        if args.no_tests && walker::is_test_file(&path) {
            continue;
        }

        let lang = match ts::Lang::from_path(&path) {
            Some(l) => l,
            None => continue,
        };

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let tree = match ts::parse_with(&mut parser, &content, lang) {
            Ok(t) => t,
            Err(_) => continue,
        };

        let imports = ts::extract_imports(&tree, &content, lang);
        let canonical_path = path.canonicalize().unwrap_or(path.clone());

        for import in &imports {
            let is_local = import.path.starts_with('.') || import.path.starts_with('/');
            if !is_local {
                continue;
            }

            if let Some(resolved) = resolve::resolve_import(&import.path, &canonical_path) {
                let resolved = resolved.canonicalize().unwrap_or(resolved);
                forward.entry(canonical_path.clone()).or_default().push(resolved.clone());
                reverse.entry(resolved).or_default().push(canonical_path.clone());
            }
        }
    }

    Ok(ImportGraph { forward, reverse })
}

/// BFS traversal rendering as a depth tree.
fn render_traversal(
    start: &Path,
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    mode: &str,
    args: &ChainArgs,
    common: &CommonArgs,
) -> Result<String> {
    let root = find_project_root(start).unwrap_or_else(|| std::env::current_dir().unwrap());
    let rel_start = pathdiff(start, &root);

    let mut output = String::new();

    if common.json {
        return render_traversal_json(start, graph, mode, args, &root);
    }

    output.push_str(&format!("Chain ({}): {}\n\n", mode, rel_start));

    // BFS with depth tracking
    let mut visited = HashSet::new();
    visited.insert(start.to_path_buf());
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();

    // Seed with direct connections
    if let Some(neighbors) = graph.get(start) {
        for n in neighbors {
            if visited.insert(n.clone()) {
                queue.push_back((n.clone(), 1));
            }
        }
    }

    let mut results: Vec<(PathBuf, usize)> = Vec::new();

    while let Some((file, depth)) = queue.pop_front() {
        if depth > args.max_depth {
            continue;
        }
        results.push((file.clone(), depth));

        if let Some(neighbors) = graph.get(&file) {
            for n in neighbors {
                if visited.insert(n.clone()) {
                    queue.push_back((n.clone(), depth + 1));
                }
            }
        }
    }

    if results.is_empty() {
        output.push_str(&format!("  (no files {} this module)\n", if mode == "reverse" { "import" } else { "imported by" }));
    } else {
        for (file, depth) in &results {
            let indent = "  ".repeat(*depth);
            let rel = pathdiff(file, &root);
            output.push_str(&format!("{}{} (depth {})\n", indent, rel, depth));
        }

        if args.skeleton {
            output.push('\n');
            output.push_str("--- Skeletons ---\n\n");
            for (file, _) in &results {
                let rel = pathdiff(file, &root);
                if let Ok(content) = fs::read_to_string(file) {
                    if let Some(lang) = ts::Lang::from_path(file) {
                        let skel = ts::extract_skeleton(&content, lang);
                        output.push_str(&format!("── {} ──\n{}\n\n", rel, skel));
                    }
                }
            }
        }

        output.push_str(&format!("\n{} files in {} chain\n", results.len(), mode));
    }

    Ok(output)
}

fn render_traversal_json(
    start: &Path,
    graph: &HashMap<PathBuf, Vec<PathBuf>>,
    mode: &str,
    args: &ChainArgs,
    root: &Path,
) -> Result<String> {
    let mut visited = HashSet::new();
    visited.insert(start.to_path_buf());
    let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();

    if let Some(neighbors) = graph.get(start) {
        for n in neighbors {
            if visited.insert(n.clone()) {
                queue.push_back((n.clone(), 1));
            }
        }
    }

    let mut entries = Vec::new();
    while let Some((file, depth)) = queue.pop_front() {
        if depth > args.max_depth {
            continue;
        }
        entries.push(serde_json::json!({
            "file": pathdiff(&file, root),
            "depth": depth,
        }));
        if let Some(neighbors) = graph.get(&file) {
            for n in neighbors {
                if visited.insert(n.clone()) {
                    queue.push_back((n.clone(), depth + 1));
                }
            }
        }
    }

    let json = serde_json::json!({
        "mode": mode,
        "target": pathdiff(start, root),
        "chain": entries,
        "count": entries.len(),
    });
    Ok(serde_json::to_string_pretty(&json)? + "\n")
}

/// BFS path-finding from `from` to `to` using forward graph.
fn render_path(
    from: &Path,
    to: &Path,
    graph: &ImportGraph,
    args: &ChainArgs,
    common: &CommonArgs,
) -> Result<String> {
    let root = find_project_root(from).unwrap_or_else(|| std::env::current_dir().unwrap());
    let rel_from = pathdiff(from, &root);
    let rel_to = pathdiff(to, &root);

    // BFS on forward graph to find shortest path
    let mut visited: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
    visited.insert(from.to_path_buf(), None);
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(from.to_path_buf());

    let mut found = false;
    while let Some(current) = queue.pop_front() {
        if current == to {
            found = true;
            break;
        }

        // Check depth limit
        let depth = path_length(&visited, &current);
        if depth >= args.max_depth {
            continue;
        }

        if let Some(neighbors) = graph.forward.get(&current) {
            for n in neighbors {
                if !visited.contains_key(n) {
                    visited.insert(n.clone(), Some(current.clone()));
                    queue.push_back(n.clone());
                }
            }
        }
    }

    if common.json {
        return render_path_json(from, to, &visited, found, &root);
    }

    let mut output = String::new();
    output.push_str(&format!("Path: {} -> {}\n\n", rel_from, rel_to));

    if !found {
        output.push_str("  (no import path found)\n");
        return Ok(output);
    }

    // Reconstruct path
    let chain = reconstruct_path(&visited, to);
    let hops = chain.len() - 1;

    for (i, file) in chain.iter().enumerate() {
        let rel = pathdiff(file, &root);
        if i == 0 {
            output.push_str(&format!("  {}\n", rel));
        } else {
            output.push_str(&format!("  -> {}\n", rel));
        }
    }

    if args.skeleton {
        output.push('\n');
        output.push_str("--- Skeletons ---\n\n");
        for file in &chain {
            let rel = pathdiff(file, &root);
            if let Ok(content) = fs::read_to_string(file) {
                if let Some(lang) = ts::Lang::from_path(file) {
                    let skel = ts::extract_skeleton(&content, lang);
                    output.push_str(&format!("── {} ──\n{}\n\n", rel, skel));
                }
            }
        }
    }

    output.push_str(&format!("\n{} files, {} hops\n", chain.len(), hops));
    Ok(output)
}

fn render_path_json(
    from: &Path,
    to: &Path,
    visited: &HashMap<PathBuf, Option<PathBuf>>,
    found: bool,
    root: &Path,
) -> Result<String> {
    let json = if found {
        let chain = reconstruct_path(visited, to);
        let files: Vec<String> = chain.iter().map(|f| pathdiff(f, root)).collect();
        serde_json::json!({
            "from": pathdiff(from, root),
            "to": pathdiff(to, root),
            "found": true,
            "path": files,
            "hops": chain.len() - 1,
        })
    } else {
        serde_json::json!({
            "from": pathdiff(from, root),
            "to": pathdiff(to, root),
            "found": false,
            "path": [],
            "hops": 0,
        })
    };
    Ok(serde_json::to_string_pretty(&json)? + "\n")
}

fn reconstruct_path(visited: &HashMap<PathBuf, Option<PathBuf>>, to: &Path) -> Vec<PathBuf> {
    let mut chain = vec![to.to_path_buf()];
    let mut current = to.to_path_buf();
    while let Some(Some(prev)) = visited.get(&current) {
        chain.push(prev.clone());
        current = prev.clone();
    }
    chain.reverse();
    chain
}

fn path_length(visited: &HashMap<PathBuf, Option<PathBuf>>, node: &Path) -> usize {
    let mut len = 0;
    let mut current = node.to_path_buf();
    while let Some(Some(prev)) = visited.get(&current) {
        len += 1;
        current = prev.clone();
    }
    len
}

/// Get a relative path string from a file to a root.
fn pathdiff(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
