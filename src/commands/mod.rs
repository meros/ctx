pub mod tree;
pub mod find;
pub mod grep;
pub mod read;
pub mod symbols;
pub mod overview;
pub mod deps;
pub mod reference;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// ctx — Context gatherer for AI-assisted planning phases.
///
/// Efficiently gathers codebase information in a single command,
/// reducing tool calls and context window usage. All operations
/// are read-only and safe to run at any time.
///
/// USAGE FOR LLMs:
///   - Use `ctx overview` first to understand a project
///   - Use `ctx grep` to find symbols/patterns across the codebase
///   - Use `ctx find` to locate files by name pattern
///   - Use `ctx read` to read multiple files in one call
///   - Use `ctx tree` to see directory structure
///   - Use `ctx symbols` to list exports/types/functions
///   - Use `ctx deps` to trace import chains
///   - Append `--ask "question"` to ANY command to filter output
///     through Claude, getting only relevant parts back
///   - Use `--json` on most commands for structured output
#[derive(Parser)]
#[command(name = "ctx", version, about, long_about)]
#[command(after_help = "QUICK REFERENCE (for LLMs):
  ctx overview .                    Project structure + README + key files
  ctx tree src/ -d 3               Directory tree, max depth 3
  ctx find '*.graphql'             Find files matching glob pattern
  ctx grep 'fetchUser' --type ts   Search for pattern in TypeScript files
  ctx read src/a.ts src/b.ts       Read multiple files, concatenated
  ctx symbols src/models/           List all exports/types/interfaces
  ctx deps src/index.ts            Show import dependency tree
  ctx grep 'TODO' --ask 'which are security-related?'
                                    LLM-filtered grep results")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show directory tree with smart filtering.
    ///
    /// Respects .gitignore, hides node_modules/dist/build by default.
    /// Output is compact — shows structure, not file contents.
    ///
    /// WHEN TO USE: Understanding project layout, finding where code lives.
    /// TYPICAL FOLLOW-UP: `ctx read` or `ctx overview` on interesting paths.
    Tree(tree::TreeArgs),

    /// Find files by name/glob pattern.
    ///
    /// Uses gitignore-aware walking. Returns file paths sorted by
    /// modification time (newest first). Supports glob patterns.
    ///
    /// WHEN TO USE: Locating files by name when you know partial names.
    /// EXAMPLE: `ctx find '*Resolver*' --type ts` finds all resolver files.
    Find(find::FindArgs),

    /// Search file contents for a pattern (regex).
    ///
    /// Shells out to ripgrep for speed. Returns matches with context.
    /// Use --type to filter by file extension.
    /// Use --fn to show the full enclosing function (replaces grep→read workflow).
    /// Use -e N to show N lines of expanded context around each match.
    ///
    /// WHEN TO USE: Finding where symbols are defined/used, reading specific methods.
    /// EXAMPLE: `ctx grep 'findPublicHomes' --fn --type ts` shows the full method.
    Grep(grep::GrepArgs),

    /// Read one or more files, concatenated with headers.
    ///
    /// Outputs each file with a clear header showing the path.
    /// Supports line ranges to read only relevant sections.
    /// Can read many files in a single invocation.
    ///
    /// WHEN TO USE: Reading file contents. Replaces multiple Read tool calls.
    /// EXAMPLE: `ctx read src/a.ts src/b.ts --lines 1-50`
    Read(read::ReadArgs),

    /// List exported symbols (functions, types, interfaces, classes).
    ///
    /// Uses ast-grep for accurate AST-based extraction when available,
    /// falls back to regex patterns. Groups by symbol kind.
    ///
    /// WHEN TO USE: Understanding a module's public API without reading full source.
    /// EXAMPLE: `ctx symbols src/models/` lists all exports in that directory.
    Symbols(symbols::SymbolsArgs),

    /// Quick project overview: structure + README + key files.
    ///
    /// Combines tree + README + package.json/Cargo.toml detection into
    /// a single output. The fastest way to understand a new project.
    ///
    /// WHEN TO USE: First command when exploring an unfamiliar project.
    Overview(overview::OverviewArgs),

    /// Show import/dependency tree for a file.
    ///
    /// Traces import statements to show what a file depends on.
    /// Helps understand coupling and find related code.
    ///
    /// WHEN TO USE: Understanding what code a file touches.
    Deps(deps::DepsArgs),

    /// Print compact LLM-optimized reference for all commands.
    ///
    /// Outputs a minimal reference card designed to be included
    /// in an LLM's system prompt or context window.
    #[command(name = "llm-reference")]
    LlmReference,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Tree(args) => tree::run(args),
        Command::Find(args) => find::run(args),
        Command::Grep(args) => grep::run(args),
        Command::Read(args) => read::run(args),
        Command::Symbols(args) => symbols::run(args),
        Command::Overview(args) => overview::run(args),
        Command::Deps(args) => deps::run(args),
        Command::LlmReference => {
            print!("{}", LLM_REFERENCE);
            Ok(())
        }
    }
}

const LLM_REFERENCE: &str = r#"# ctx — Context Gatherer CLI Reference

Read-only codebase exploration tool. All commands are safe. Use `--ask "Q"` on any command to filter output through Claude.

## Commands

| Command | Purpose | Example |
|---------|---------|---------|
| overview [path] | Project structure + README + key config | `ctx overview .` |
| tree [path] [-d N] | Directory tree (respects .gitignore) | `ctx tree src/ -d 3` |
| find <glob> [--type ext] | Find files by name pattern | `ctx find '*Model*' --type ts` |
| grep <pattern> [path] [--type ext] [-C N] | Search contents (regex) | `ctx grep 'fetchUser' --type ts` |
| grep <pattern> --fn | Search + show full enclosing function | `ctx grep 'fetchUser' --fn` |
| grep <pattern> -e 80 | Search + show N lines of context | `ctx grep 'fetchUser' -e 80` |
| read <files...> [--lines N-M] | Read multiple files | `ctx read a.ts b.ts --lines 1-50` |
| symbols <path> [--kind fn\|type\|class] | List exports/types/functions | `ctx symbols src/models/` |
| deps <file> | Import dependency tree | `ctx deps src/index.ts` |

## Global Options

| Option | Effect |
|--------|--------|
| --ask "question" | Filter output through Claude (smart extraction) |
| --json | Machine-readable JSON output |
| --no-gitignore | Include gitignored files |

## Decision Guide

- New project? → `ctx overview .`
- Know the name? → `ctx find '*name*'`
- Know a symbol? → `ctx grep 'symbol'` (add `--fn` to see full function body)
- Need file contents? → `ctx read file1 file2`
- Need API surface? → `ctx symbols path/`
- Need dependency chain? → `ctx deps file`
"#;
