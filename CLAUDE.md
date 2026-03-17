# ctx — Context Gatherer CLI

## What This Is

A Rust CLI tool that efficiently gathers codebase context for AI-assisted planning phases. Reduces the number of tool calls and context window usage by bundling common exploration patterns into single commands.

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
- `src/commands/mod.rs` — CLI definition (clap derive), LLM reference card
- `src/commands/{tree,find,grep,read,symbols,overview,deps}.rs` — Individual commands
- `src/filter.rs` — `--ask` flag implementation (pipes through `claude -p`)

## Key Design Decisions

- **Read-only**: All commands are safe, no writes. Designed for the PLAN phase.
- **Self-documenting**: `ctx llm-reference` outputs a compact reference card for LLM context. Each subcommand has rich `--help` with "WHEN TO USE" guidance.
- **`--ask` flag**: Any command can pipe its output through Claude for smart filtering/summarization, keeping the main conversation context clean.
- **Shells out to power tools**: Uses `rg` (ripgrep), `fd`, `ast-grep` when available, with built-in fallbacks.
- **Respects .gitignore**: All file walking uses the `ignore` crate.

## Testing

```bash
cargo run -- overview .
cargo run -- tree src/ -d 3
cargo run -- grep 'pattern' --type rs
cargo run -- find '*.rs'
cargo run -- symbols src/ --type rs
```
