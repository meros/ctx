pub mod tree;
pub mod find;
pub mod grep;
pub mod read;
pub mod symbols;
pub mod overview;
pub mod deps;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

/// ctx — Context gatherer for AI-assisted planning phases.
///
/// Efficiently gathers codebase information in a single command,
/// reducing tool calls and context window usage. All operations
/// are read-only and safe to run at any time.
///
/// DECISION GUIDE — which command do I need?
///
///   New/unfamiliar project?    → ctx overview .
///   Know a file name?          → ctx find '*name*'
///   Know a symbol/pattern?     → ctx grep 'symbol' (add --fn for full function body)
///   Need file contents?        → ctx read file1 file2
///   Need a module's API?       → ctx symbols path/
///   Need dependency chain?     → ctx deps file
///   Need directory layout?     → ctx tree path/ -d 3
///
/// GLOBAL OPTIONS (available on all commands, can go before or after subcommand):
///
///   --ask "question"   Filter output through Claude (smart extraction)
///   --tokens N         Truncate output to ~N estimated tokens
///   --json             Machine-readable JSON output
///   --no-gitignore     Include gitignored files
#[derive(Parser)]
#[command(name = "ctx", version, about, long_about)]
#[command(after_help = "EXAMPLES:
  ctx overview .                               Understand a project quickly
  ctx tree src/ -d 3                           Directory tree, max depth 3
  ctx find '*.graphql'                         Find files matching glob pattern
  ctx grep 'fetchUser' --type ts               Search TypeScript files for pattern
  ctx grep 'fetchUser' --fn --no-tests         Show full function, skip test files
  ctx grep 'foo|bar' --fn                      Alternation (use | not \\|)
  ctx grep 'TODO' --ask 'which are security-related?'
                                               LLM-filtered grep results
  ctx read src/a.ts src/b.ts                   Read multiple files, concatenated
  ctx read src/a.ts --lines 10-50              Read specific line range
  ctx symbols src/models/                      List all exports/types/interfaces
  ctx deps src/index.ts                        Show import dependency tree

FLAGS CAN GO ANYWHERE:
  ctx --tokens 500 grep 'pattern'              Before subcommand
  ctx grep 'pattern' --tokens 500              After subcommand
  ctx grep --tokens 500 'pattern' --fn         Mixed with subcommand flags

TYPICAL WORKFLOW:
  1. ctx overview .                  Get the lay of the land
  2. ctx grep 'relevantSymbol' --fn  Find and read the code you need
  3. ctx deps src/target.ts          Understand what it touches
  4. ctx read src/a.ts src/b.ts      Read related files in one call")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// Global options available on all subcommands.
#[derive(Args, Debug)]
pub struct CommonArgs {
    /// Filter output through Claude with a question
    #[arg(long, global = true)]
    pub ask: Option<String>,

    /// Maximum output size in estimated tokens (truncates with notice)
    #[arg(long, global = true)]
    pub tokens: Option<usize>,

    /// Output as JSON
    #[arg(long, global = true)]
    pub json: bool,

    /// Include gitignored files
    #[arg(long = "no-gitignore", global = true)]
    pub no_gitignore: bool,
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
    ///
    /// EXAMPLES:
    ///   ctx tree src/ -d 3          Show src/ tree, max 3 levels deep
    ///   ctx tree . -d 2 --json      Machine-readable directory structure
    Tree(tree::TreeArgs),

    /// Find files by name/glob pattern.
    ///
    /// Uses gitignore-aware walking. Returns file paths sorted by
    /// modification time (newest first). Supports glob patterns.
    ///
    /// WHEN TO USE: Locating files by name when you know partial names.
    ///
    /// EXAMPLES:
    ///   ctx find '*.graphql'            All GraphQL schema files
    ///   ctx find '*Resolver*' --type ts  TypeScript resolver files
    ///   ctx find 'Dockerfile*'          All Dockerfiles
    Find(find::FindArgs),

    /// Search file contents for a pattern (regex).
    ///
    /// Uses Rust regex syntax (same as ripgrep). Returns matches with context.
    /// Use --type to filter by file extension.
    /// Use --fn to show the full enclosing function (replaces grep→read workflow).
    /// Use --no-tests to exclude test files from results.
    /// Use -e N to show N lines of expanded context around each match.
    ///
    /// REGEX QUICK REFERENCE (Rust/ripgrep syntax):
    ///   foo|bar        Alternation (match foo OR bar)
    ///   (group)        Grouping (NOT \( \) like BRE)
    ///   \bword\b       Word boundary
    ///   (?i)pattern    Case-insensitive (or use -i flag)
    ///   foo.*bar       foo followed by bar on same line
    ///
    /// NOTE: This is NOT GNU grep. Don't use \| \( \) — those match literals.
    /// Common BRE patterns are auto-converted with a warning.
    ///
    /// WHEN TO USE: Finding where symbols are defined/used, reading specific methods.
    ///
    /// EXAMPLES:
    ///   ctx grep 'fetchUser' --type ts          Find all references in TS files
    ///   ctx grep 'fetch|send' --fn              Multiple patterns with alternation
    ///   ctx grep 'fetchUser' --fn --no-tests    Show full function, skip tests
    ///   ctx grep 'TODO' -e 5                    TODOs with 5 lines of context
    ///   ctx grep 'dbConnect' --ask 'which handle errors?'
    ///                                           LLM-filtered results
    Grep(grep::GrepArgs),

    /// Read one or more files, concatenated with headers.
    ///
    /// Outputs each file with a clear header showing the path.
    /// Supports line ranges to read only relevant sections.
    /// Can read many files in a single invocation.
    ///
    /// WHEN TO USE: Reading file contents. Replaces multiple Read tool calls.
    ///
    /// EXAMPLES:
    ///   ctx read src/a.ts src/b.ts        Read two files in one call
    ///   ctx read src/a.ts --lines 10-50   Read specific line range
    Read(read::ReadArgs),

    /// List exported symbols (functions, types, interfaces, classes).
    ///
    /// Uses ast-grep for accurate AST-based extraction when available,
    /// falls back to regex patterns. Groups by symbol kind.
    ///
    /// WHEN TO USE: Understanding a module's public API without reading full source.
    ///
    /// EXAMPLES:
    ///   ctx symbols src/models/                 All exports in directory
    ///   ctx symbols src/ --type rs --kind fn    Only Rust functions
    Symbols(symbols::SymbolsArgs),

    /// Quick project overview: structure + README + key files.
    ///
    /// Combines tree + README + package.json/Cargo.toml detection into
    /// a single output. The fastest way to understand a new project.
    ///
    /// WHEN TO USE: First command when exploring an unfamiliar project.
    /// This should almost always be your first ctx command.
    ///
    /// EXAMPLES:
    ///   ctx overview .                  Overview of current project
    ///   ctx overview packages/api/      Overview of a subpackage
    Overview(overview::OverviewArgs),

    /// Show import/dependency tree for a file.
    ///
    /// Traces import statements to show what a file depends on.
    /// Helps understand coupling and find related code.
    ///
    /// WHEN TO USE: Understanding what code a file touches, tracing coupling.
    ///
    /// EXAMPLES:
    ///   ctx deps src/index.ts           Show what index.ts imports
    ///   ctx deps src/api/router.ts      Trace API router dependencies
    Deps(deps::DepsArgs),
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Tree(args) => tree::run(args, &cli.common),
        Command::Find(args) => find::run(args, &cli.common),
        Command::Grep(args) => grep::run(args, &cli.common),
        Command::Read(args) => read::run(args, &cli.common),
        Command::Symbols(args) => symbols::run(args, &cli.common),
        Command::Overview(args) => overview::run(args, &cli.common),
        Command::Deps(args) => deps::run(args, &cli.common),
    }
}
