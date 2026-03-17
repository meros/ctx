use anyhow::Result;
use clap::Args;
use ignore::WalkBuilder;
use std::fs;
use std::path::PathBuf;

use crate::filter;
use crate::ts;

#[derive(Args)]
pub struct SymbolsArgs {
    /// File or directory to extract symbols from
    pub path: PathBuf,

    /// Filter by symbol kind (fn, type, class, interface, const, enum, struct, trait, impl, def)
    #[arg(short = 'k', long = "kind")]
    pub kind: Option<String>,

    /// Filter by file extension (e.g., ts, rs, py)
    #[arg(short = 't', long = "type")]
    pub file_type: Option<String>,

    /// Filter output through Claude with a question
    #[arg(long)]
    pub ask: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: SymbolsArgs) -> Result<()> {
    let files = collect_files(&args)?;
    let mut all_symbols: Vec<(String, ts::SymbolInfo)> = Vec::new();

    for file in &files {
        let lang = match ts::Lang::from_path(file) {
            Some(l) => l,
            None => continue,
        };

        let content = match fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let tree = match ts::parse(&content, lang) {
            Ok(t) => t,
            Err(_) => continue,
        };

        let symbols = ts::extract_symbols(&tree, &content, lang);
        for sym in symbols {
            // Apply kind filter
            if let Some(ref kind_filter) = args.kind {
                if !sym.kind.contains(kind_filter.as_str()) {
                    continue;
                }
            }
            all_symbols.push((file.to_string_lossy().to_string(), sym));
        }
    }

    let output = if args.json {
        let json: Vec<serde_json::Value> = all_symbols
            .iter()
            .map(|(file, sym)| {
                serde_json::json!({
                    "file": file,
                    "name": sym.name,
                    "kind": sym.kind,
                    "line": sym.line + 1,
                    "signature": sym.signature,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json)?
    } else {
        let mut out = String::new();
        let mut current_file = String::new();
        for (file, sym) in &all_symbols {
            if *file != current_file {
                if !current_file.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("=== {} ===\n", file));
                current_file = file.clone();
            }
            out.push_str(&format!(
                "  {:>4}  {:>10}  {}: {}\n",
                sym.line + 1,
                sym.kind,
                sym.name,
                sym.signature
            ));
        }
        if all_symbols.is_empty() {
            "No symbols found.\n".to_string()
        } else {
            out
        }
    };

    let output = filter::maybe_filter(&output, &args.ask)?;
    print!("{}", output);
    Ok(())
}

fn collect_files(args: &SymbolsArgs) -> Result<Vec<PathBuf>> {
    if args.path.is_file() {
        return Ok(vec![args.path.clone()]);
    }

    let mut files = Vec::new();
    let walker = WalkBuilder::new(&args.path)
        .git_ignore(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "node_modules" | ".git" | "dist" | "build" | ".next" | "__pycache__" | "target"
            )
        })
        .build();

    for entry in walker.flatten() {
        let path = entry.path().to_path_buf();
        if !path.is_file() {
            continue;
        }

        // Filter by extension if specified
        if let Some(ref ft) = args.file_type {
            if path.extension().and_then(|e| e.to_str()) != Some(ft.as_str()) {
                continue;
            }
        }

        // Only include files tree-sitter can parse
        if ts::Lang::from_path(&path).is_some() {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}
