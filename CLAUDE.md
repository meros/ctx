# ctx — Context Gatherer CLI

## What This Is

A Rust CLI tool that efficiently gathers codebase context for AI-assisted planning phases. Reduces the number of tool calls and context window usage by bundling common exploration patterns into single commands.

## Documentation

The CLI is fully self-documenting. Run `ctx --help` for a complete decision guide and examples, or `ctx <command> --help` for detailed per-command usage.

## Build

```bash
# Development
nix develop  # or: nix-shell -p cargo rustc
cargo build

# Release
cargo build --release
```

## Architecture

- `src/main.rs` — Entry point
- `src/commands/mod.rs` — CLI definition (clap derive), help text
- `src/commands/{tree,find,grep,read,symbols,overview,deps}.rs` — Individual commands
- `src/walker.rs` — Shared gitignore-aware file walker with noise directory filtering
- `src/ts.rs` — Tree-sitter integration (parsing, symbol extraction, import analysis, function detection)
- `src/filter.rs` — `--ask` flag implementation (pipes through `claude -p`)

## Key Design Decisions

- **Read-only**: All commands are safe, no writes. Designed for the PLAN phase.
- **Self-documenting**: `ctx --help` and `ctx <cmd> --help` are the complete documentation. No separate reference card needed.
- **Global flags**: `--ask`, `--tokens`, `--json`, `--no-gitignore` work on all subcommands and can be placed anywhere in the command line (before or after the subcommand).
- **BRE auto-conversion**: `ctx grep` uses Rust/ripgrep regex syntax. Common BRE patterns (`\|`, `\(`, `\)`, `\+`, `\?`) are auto-detected and converted with a stderr warning. Invalid patterns show a helpful syntax quick-reference.
- **Native Rust implementation**: Uses `grep-regex`/`grep-searcher` (ripgrep's libraries) and `tree-sitter` directly — no external tool dependencies.
- **Respects .gitignore**: All file walking uses the `ignore` crate.

## Testing

```bash
cargo run -- overview .
cargo run -- tree src/ -d 3
cargo run -- grep 'pattern' --type rs
cargo run -- find '*.rs'
cargo run -- symbols src/ --type rs
```
