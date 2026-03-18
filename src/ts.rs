//! Tree-sitter integration for AST-based code analysis.
//!
//! Provides language detection, parsing, and common queries:
//! - Find enclosing function at a line number
//! - Extract imports from a file
//! - List exported symbols

use anyhow::{Context, Result};
use std::path::Path;
use tree_sitter::{Language, Node, Parser, Tree};

/// Supported languages for tree-sitter parsing.
#[derive(Debug, Clone, Copy)]
pub enum Lang {
    TypeScript,
    Tsx,
    JavaScript,
    Rust,
    Python,
}

impl Lang {
    /// Detect language from file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" => Some(Lang::Tsx),
            "js" | "mjs" | "cjs" | "jsx" => Some(Lang::JavaScript),
            "rs" => Some(Lang::Rust),
            "py" | "pyi" => Some(Lang::Python),
            _ => None,
        }
    }

    fn tree_sitter_language(self) -> Language {
        match self {
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    /// Node kinds that represent function-like constructs in this language.
    fn function_node_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => &[
                "function_declaration",
                "method_definition",
                "arrow_function",
                "function_expression",
                "generator_function_declaration",
            ],
            Lang::Rust => &[
                "function_item",
            ],
            Lang::Python => &[
                "function_definition",
                "decorated_definition",
            ],
        }
    }

    /// Node kinds that represent import statements.
    fn import_node_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => &[
                "import_statement",
                "export_statement",
            ],
            Lang::Rust => &[
                "use_declaration",
                "mod_item",
                "extern_crate_declaration",
            ],
            Lang::Python => &[
                "import_statement",
                "import_from_statement",
            ],
        }
    }

}

/// Parse source code with tree-sitter.
pub fn parse(source: &str, lang: Lang) -> Result<Tree> {
    let mut parser = Parser::new();
    parse_with(&mut parser, source, lang)
}

/// Parse source code reusing an existing Parser instance.
/// More efficient when parsing multiple files — avoids re-allocating the parser each time.
pub fn parse_with(parser: &mut Parser, source: &str, lang: Lang) -> Result<Tree> {
    parser
        .set_language(&lang.tree_sitter_language())
        .context("Failed to set tree-sitter language")?;
    parser
        .parse(source, None)
        .context("Failed to parse source code")
}

/// Find the enclosing function/method node at a given line (0-indexed).
/// Returns (start_line, end_line) inclusive, both 0-indexed.
pub fn find_enclosing_function(tree: &Tree, source: &str, target_line: usize, lang: Lang) -> Option<(usize, usize)> {
    let root = tree.root_node();
    let function_kinds = lang.function_node_kinds();

    // Find the smallest function node that contains the target line
    let mut best: Option<Node> = None;

    find_enclosing_node_recursive(root, target_line, function_kinds, &mut best);

    // If we found a function, check for decorators/export wrappers above it
    if let Some(node) = best {
        let mut start = node.start_position().row;
        let end = node.end_position().row;

        // Walk up to include export_statement or decorated_definition wrapper
        if let Some(parent) = node.parent() {
            let parent_kind = parent.kind();
            if parent_kind == "export_statement"
                || parent_kind == "decorated_definition"
                || parent_kind == "lexical_declaration"
            {
                start = parent.start_position().row;
            }
        }

        // Include leading comments (walk backwards from start)
        let lines: Vec<&str> = source.lines().collect();
        while start > 0 {
            let prev = lines.get(start - 1).map(|l| l.trim()).unwrap_or("");
            if prev.starts_with("//")
                || prev.starts_with("/*")
                || prev.starts_with('*')
                || prev.starts_with("*/")
                || prev.starts_with('#')
                || prev.starts_with("///")
            {
                start -= 1;
            } else {
                break;
            }
        }

        Some((start, end))
    } else {
        None
    }
}

fn find_enclosing_node_recursive<'a>(
    node: Node<'a>,
    target_line: usize,
    kinds: &[&str],
    best: &mut Option<Node<'a>>,
) {
    let start = node.start_position().row;
    let end = node.end_position().row;

    if target_line < start || target_line > end {
        return;
    }

    if kinds.contains(&node.kind()) {
        // This node contains the target line — update best if it's smaller
        match best {
            Some(current) => {
                let current_size = current.end_position().row - current.start_position().row;
                let new_size = end - start;
                if new_size < current_size {
                    *best = Some(node);
                }
            }
            None => *best = Some(node),
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_enclosing_node_recursive(child, target_line, kinds, best);
    }
}

/// Extract import information from a parsed file.
pub struct ImportInfo {
    pub path: String,
    pub kind: String,
    pub line: usize,
}

pub fn extract_imports(tree: &Tree, source: &str, lang: Lang) -> Vec<ImportInfo> {
    let root = tree.root_node();
    let import_kinds = lang.import_node_kinds();
    let mut imports = Vec::new();

    collect_imports_recursive(root, source, import_kinds, lang, &mut imports);
    imports
}

fn collect_imports_recursive(
    node: Node,
    source: &str,
    kinds: &[&str],
    lang: Lang,
    imports: &mut Vec<ImportInfo>,
) {
    if kinds.contains(&node.kind()) {
        if let Some(import) = extract_import_path(node, source, lang) {
            imports.push(import);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports_recursive(child, source, kinds, lang, imports);
    }
}

fn extract_import_path(node: Node, source: &str, lang: Lang) -> Option<ImportInfo> {
    let line = node.start_position().row;
    let text = node.utf8_text(source.as_bytes()).ok()?;

    match lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            // Only extract path from import/export statements that have a source
            // (i.e., `import X from '...'` or `export { X } from '...'`)
            // Skip `export const X = ...` which has no source string
            let has_source = node.child_by_field_name("source").is_some();
            if !has_source && node.kind() == "export_statement" {
                return None;
            }
            let kind = if text.starts_with("export") { "re-export" } else { "import" };
            // Look for the source string (the module path)
            let source_node = node.child_by_field_name("source");
            let path = if let Some(src) = source_node {
                // Get the string content without quotes
                let src_text = src.utf8_text(source.as_bytes()).ok()?;
                src_text.trim_matches(|c| c == '\'' || c == '"').to_string()
            } else {
                find_string_in_node(node, source)?
            };
            Some(ImportInfo {
                path,
                kind: kind.to_string(),
                line,
            })
        }
        Lang::Rust => {
            let kind = node.kind();
            // For use declarations, get the scoped identifier
            let text = text.trim_end_matches(';').trim();
            let path = if text.starts_with("use ") {
                text.strip_prefix("use ")?.to_string()
            } else if text.starts_with("mod ") {
                text.strip_prefix("mod ")?.to_string()
            } else if text.starts_with("extern crate ") {
                text.strip_prefix("extern crate ")?.to_string()
            } else {
                return None;
            };
            Some(ImportInfo {
                path,
                kind: kind.to_string(),
                line,
            })
        }
        Lang::Python => {
            // import X or from X import Y
            let kind = node.kind();
            // Extract module name from the node
            if let Some(module) = find_child_by_kind(node, "dotted_name") {
                let path = module.utf8_text(source.as_bytes()).ok()?.to_string();
                Some(ImportInfo {
                    path,
                    kind: kind.to_string(),
                    line,
                })
            } else {
                None
            }
        }
    }
}

fn find_string_in_node(node: Node, source: &str) -> Option<String> {
    // Look for string/string_fragment children recursively
    if node.kind() == "string_fragment" || node.kind() == "string_content" {
        return node.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
    }
    if node.kind() == "string" {
        // Get the text without quotes
        let text = node.utf8_text(source.as_bytes()).ok()?;
        return Some(text.trim_matches(|c| c == '\'' || c == '"').to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_string_in_node(child, source) {
            return Some(found);
        }
    }
    None
}

fn find_child_by_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = find_child_by_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

/// A symbol extracted from source code.
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub signature: String,
}

/// Extract exported/public symbols from a parsed file.
pub fn extract_symbols(tree: &Tree, source: &str, lang: Lang) -> Vec<SymbolInfo> {
    let root = tree.root_node();
    let mut symbols = Vec::new();

    match lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            collect_ts_symbols(root, source, &mut symbols);
        }
        Lang::Rust => {
            collect_rust_symbols(root, source, &mut symbols);
        }
        Lang::Python => {
            collect_python_symbols(root, source, &mut symbols);
        }
    }

    symbols
}

fn collect_ts_symbols(node: Node, source: &str, symbols: &mut Vec<SymbolInfo>) {
    if node.kind() == "export_statement" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            match kind {
                "function_declaration" | "generator_function_declaration" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                        let sig = first_line(child, source);
                        symbols.push(SymbolInfo { name, kind: "function".into(), line: child.start_position().row, signature: sig });
                    }
                }
                "lexical_declaration" => {
                    let mut inner = child.walk();
                    for decl in child.children(&mut inner) {
                        if decl.kind() == "variable_declarator" {
                            if let Some(name_node) = decl.child_by_field_name("name") {
                                let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                                let sig = first_line(node, source);
                                symbols.push(SymbolInfo { name, kind: "const".into(), line: decl.start_position().row, signature: sig });
                            }
                        }
                    }
                }
                "class_declaration" | "abstract_class_declaration" => {
                    let class_kind = if kind == "abstract_class_declaration" { "abstract class" } else { "class" };
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                        let sig = first_line(child, source);
                        symbols.push(SymbolInfo { name, kind: class_kind.into(), line: child.start_position().row, signature: sig });
                    }
                    // Recurse into class body to extract methods
                    collect_ts_class_members(child, source, symbols);
                }
                "type_alias_declaration" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                        let sig = first_line(child, source);
                        symbols.push(SymbolInfo { name, kind: "type".into(), line: child.start_position().row, signature: sig });
                    }
                }
                "interface_declaration" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                        let sig = first_line(child, source);
                        symbols.push(SymbolInfo { name, kind: "interface".into(), line: child.start_position().row, signature: sig });
                    }
                }
                "enum_declaration" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                        let sig = first_line(child, source);
                        symbols.push(SymbolInfo { name, kind: "enum".into(), line: child.start_position().row, signature: sig });
                    }
                }
                _ => {}
            }
        }
    }

    // Also handle `export default class ...` at module level
    if node.kind() == "export_statement" {
        // Check for default export of a class
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        if text.starts_with("export default class") {
            if let Some(class_node) = find_child_by_kind(node, "class") {
                let name = class_node
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("default")
                    .to_string();
                let sig = first_line(class_node, source);
                // Only push if not already added above
                if !symbols.iter().any(|s| s.line == class_node.start_position().row) {
                    symbols.push(SymbolInfo { name, kind: "class".into(), line: class_node.start_position().row, signature: sig });
                }
                collect_ts_class_members(class_node, source, symbols);
            }
        }
    }

    // Recurse into children (skip function bodies but DO enter class bodies for default exports)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "statement_block" {
            collect_ts_symbols(child, source, symbols);
        }
    }
}

/// Extract methods and properties from a class body.
fn collect_ts_class_members(class_node: Node, source: &str, symbols: &mut Vec<SymbolInfo>) {
    let body = match find_child_by_kind(class_node, "class_body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for member in body.children(&mut cursor) {
        match member.kind() {
            "method_definition" | "public_field_definition" => {
                let name = member
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("?")
                    .to_string();

                // Determine if static
                let is_static = has_child_kind(member, "static");
                let kind_prefix = if is_static { "static " } else { "" };
                let kind_suffix = if member.kind() == "method_definition" {
                    "method"
                } else {
                    "field"
                };

                let sig = first_line(member, source);
                symbols.push(SymbolInfo {
                    name,
                    kind: format!("{}{}", kind_prefix, kind_suffix),
                    line: member.start_position().row,
                    signature: sig,
                });
            }
            "method_signature" => {
                let name = member
                    .child_by_field_name("name")
                    .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                    .unwrap_or("?")
                    .to_string();
                let sig = first_line(member, source);
                symbols.push(SymbolInfo {
                    name,
                    kind: "method".into(),
                    line: member.start_position().row,
                    signature: sig,
                });
            }
            _ => {}
        }
    }
}

fn has_child_kind(node: Node, kind: &str) -> bool {
    let mut cursor = node.walk();
    let result = node.children(&mut cursor).any(|c| c.kind() == kind);
    result
}

fn collect_rust_symbols(node: Node, source: &str, symbols: &mut Vec<SymbolInfo>) {
    let kind = node.kind();
    let is_pub = node
        .child(0)
        .map(|c| c.kind() == "visibility_modifier")
        .unwrap_or(false);

    if is_pub {
        match kind {
            "function_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                    let sig = first_line(node, source);
                    symbols.push(SymbolInfo { name, kind: "fn".into(), line: node.start_position().row, signature: sig });
                }
            }
            "struct_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                    let sig = first_line(node, source);
                    symbols.push(SymbolInfo { name, kind: "struct".into(), line: node.start_position().row, signature: sig });
                }
            }
            "enum_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                    let sig = first_line(node, source);
                    symbols.push(SymbolInfo { name, kind: "enum".into(), line: node.start_position().row, signature: sig });
                }
            }
            "trait_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                    let sig = first_line(node, source);
                    symbols.push(SymbolInfo { name, kind: "trait".into(), line: node.start_position().row, signature: sig });
                }
            }
            "type_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                    let sig = first_line(node, source);
                    symbols.push(SymbolInfo { name, kind: "type".into(), line: node.start_position().row, signature: sig });
                }
            }
            _ => {}
        }
    }

    // Also collect impl blocks
    if kind == "impl_item" {
        if let Some(name_node) = node.child_by_field_name("type") {
            let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
            let sig = first_line(node, source);
            symbols.push(SymbolInfo { name, kind: "impl".into(), line: node.start_position().row, signature: sig });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_symbols(child, source, symbols);
    }
}

fn collect_python_symbols(node: Node, source: &str, symbols: &mut Vec<SymbolInfo>) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                // Only top-level or class-level
                if is_top_level_or_class_method(node) {
                    let sig = first_line(node, source);
                    symbols.push(SymbolInfo { name, kind: "def".into(), line: node.start_position().row, signature: sig });
                }
            }
        }
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = name_node.utf8_text(source.as_bytes()).unwrap_or("?").to_string();
                let sig = first_line(node, source);
                symbols.push(SymbolInfo { name, kind: "class".into(), line: node.start_position().row, signature: sig });
            }
        }
        "decorated_definition" => {
            // Recurse to get the inner function/class
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_python_symbols(child, source, symbols);
            }
            return; // Don't double-recurse
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_symbols(child, source, symbols);
    }
}

fn is_top_level_or_class_method(node: Node) -> bool {
    if let Some(parent) = node.parent() {
        matches!(
            parent.kind(),
            "module" | "source_file" | "class_body" | "block" | "decorated_definition"
        )
    } else {
        true
    }
}

/// Extract a skeleton view of source code — signatures, types, imports only.
/// Replaces function/method bodies with `{ ... }` (or `...` for Python) to dramatically
/// reduce tokens while preserving the module's API surface.
pub fn extract_skeleton(source: &str, lang: Lang) -> String {
    let tree = match parse(source, lang) {
        Ok(t) => t,
        Err(_) => return source.to_string(),
    };

    let root = tree.root_node();
    let lines: Vec<&str> = source.lines().collect();
    let mut output = String::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        skeleton_node(child, source, &lines, lang, &mut output, 0);
    }

    // Remove trailing blank lines but keep final newline
    let trimmed = output.trim_end();
    if trimmed.is_empty() {
        output
    } else {
        let mut result = trimmed.to_string();
        result.push('\n');
        result
    }
}

/// Recursively emit skeleton for a node.
/// `depth` is the nesting level for indentation context.
fn skeleton_node(
    node: Node,
    source: &str,
    lines: &[&str],
    lang: Lang,
    output: &mut String,
    depth: usize,
) {
    match lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
            skeleton_node_ts(node, source, lines, lang, output, depth);
        }
        Lang::Rust => {
            skeleton_node_rust(node, source, lines, lang, output, depth);
        }
        Lang::Python => {
            skeleton_node_python(node, source, lines, lang, output, depth);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn skeleton_node_ts(
    node: Node,
    source: &str,
    lines: &[&str],
    lang: Lang,
    output: &mut String,
    depth: usize,
) {
    let kind = node.kind();
    let start_row = node.start_position().row;
    let end_row = node_last_row(node);
    match kind {
        // Function-like: emit signature + { ... }
        "function_declaration" | "generator_function_declaration" | "function_expression" => {
            emit_signature_then_body_placeholder(node, source, lines, output, "{", "{ ... }");
        }
        "arrow_function" => {
            // Arrow functions: find the => and body
            emit_signature_then_body_placeholder(node, source, lines, output, "=>", "=> { ... }");
        }
        "method_definition" => {
            emit_signature_then_body_placeholder(node, source, lines, output, "{", "{ ... }");
        }
        // Export: recurse into children to handle the inner declaration
        "export_statement" => {
            // Check if this is a simple re-export (has source) or export of a declaration
            let has_declaration = node.named_child_count() > 0
                && node.named_child(0).map(|c| matches!(c.kind(),
                    "function_declaration" | "class_declaration" | "abstract_class_declaration"
                    | "lexical_declaration" | "type_alias_declaration" | "interface_declaration"
                    | "enum_declaration" | "generator_function_declaration"
                )).unwrap_or(false);

            if has_declaration {
                let inner = node.named_child(0).unwrap();
                let inner_start = inner.start_position().row;
                if inner_start == start_row {
                    // Same line: the full source line already contains "export",
                    // so just skeleton the inner node (which uses line-based output)
                    skeleton_node(inner, source, lines, lang, output, depth);
                } else {
                    // Different lines — emit export lines then recurse
                    for line in &lines[start_row..inner_start] {
                        output.push_str(line);
                        output.push('\n');
                    }
                    skeleton_node(inner, source, lines, lang, output, depth);
                }
            } else {
                // Simple export (re-export, export clause, etc.) — emit as-is
                emit_node_full(lines, start_row, end_row, output);
            }
        }
        // Lexical declarations (const/let/var): check if value is an arrow function
        "lexical_declaration" => {
            // Check if any variable_declarator has an arrow_function value
            let mut cursor = node.walk();
            let has_arrow = node.children(&mut cursor).any(|c| {
                c.kind() == "variable_declarator"
                    && c.child_by_field_name("value")
                        .map(|v| v.kind() == "arrow_function")
                        .unwrap_or(false)
            });

            if has_arrow {
                emit_signature_then_body_placeholder(node, source, lines, output, "=>", "=> { ... }");
            } else {
                emit_node_full(lines, start_row, end_row, output);
            }
        }
        // Class: emit class line then skeleton of members
        "class_declaration" | "abstract_class_declaration" => {
            // Find the class_body child
            if let Some(body) = find_child_by_kind(node, "class_body") {
                // Emit everything from class start to body open brace
                let body_start = body.start_position().row;
                for line in &lines[start_row..=body_start] {
                    output.push_str(line);
                    output.push('\n');
                }
                // Recurse into class body members
                let mut cursor = body.walk();
                for member in body.children(&mut cursor) {
                    if member.is_named() {
                        skeleton_node(member, source, lines, lang, output, depth + 1);
                    }
                }
                // Close brace
                let body_end = node_last_row(body);
                output.push_str(lines[body_end]);
                output.push('\n');
            } else {
                emit_node_full(lines, start_row, end_row, output);
            }
        }
        // Types/interfaces/enums: keep fully (they're already compact API surface)
        "type_alias_declaration" | "interface_declaration" | "enum_declaration" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Import statements: keep as-is
        "import_statement" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Comments: keep
        "comment" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Everything else at this level: emit as-is
        _ => {
            emit_node_full(lines, start_row, end_row, output);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn skeleton_node_rust(
    node: Node,
    source: &str,
    lines: &[&str],
    lang: Lang,
    output: &mut String,
    depth: usize,
) {
    let kind = node.kind();
    let start_row = node.start_position().row;
    let end_row = node_last_row(node);
    match kind {
        // fn items: signature + { ... }
        "function_item" => {
            emit_signature_then_body_placeholder(node, source, lines, output, "{", "{ ... }");
        }
        // impl blocks: impl line + skeleton of methods
        "impl_item" => {
            if let Some(body) = find_child_by_kind(node, "declaration_list") {
                let body_start = body.start_position().row;
                // Emit from impl start to opening brace
                for line in &lines[start_row..=body_start] {
                    output.push_str(line);
                    output.push('\n');
                }
                // Recurse into impl methods
                let mut cursor = body.walk();
                for member in body.children(&mut cursor) {
                    if member.is_named() {
                        skeleton_node(member, source, lines, lang, output, depth + 1);
                    }
                }
                // Closing brace
                let body_end = node_last_row(body);
                output.push_str(lines[body_end]);
                output.push('\n');
            } else {
                emit_node_full(lines, start_row, end_row, output);
            }
        }
        // trait: keep fully (signatures are the API)
        "trait_item" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // struct/enum: keep fully (fields are the API)
        "struct_item" | "enum_item" | "type_item" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // use/mod/extern: keep as-is
        "use_declaration" | "mod_item" | "extern_crate_declaration" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Attribute: keep (often #[derive(...)] etc.)
        "attribute_item" | "inner_attribute_item" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Comments and doc comments
        "line_comment" | "block_comment" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Macro invocations: keep as-is
        "macro_invocation" | "macro_definition" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // const/static items: keep as-is
        "const_item" | "static_item" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        _ => {
            emit_node_full(lines, start_row, end_row, output);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn skeleton_node_python(
    node: Node,
    source: &str,
    lines: &[&str],
    lang: Lang,
    output: &mut String,
    depth: usize,
) {
    let kind = node.kind();
    let start_row = node.start_position().row;
    let end_row = node_last_row(node);
    match kind {
        "function_definition" => {
            // Emit def line(s) + indented ...
            emit_python_def_skeleton(node, lines, output);
        }
        "class_definition" => {
            // Emit class line, then skeleton of methods
            if let Some(body) = node.child_by_field_name("body") {
                let body_start = body.start_position().row;
                // Emit from class start to body start
                for line in &lines[start_row..body_start] {
                    output.push_str(line);
                    output.push('\n');
                }
                // Recurse into class body members
                let mut cursor = body.walk();
                for member in body.children(&mut cursor) {
                    if member.is_named() {
                        skeleton_node(member, source, lines, lang, output, depth + 1);
                    }
                }
            } else {
                emit_node_full(lines, start_row, end_row, output);
            }
        }
        "decorated_definition" => {
            // Emit decorator lines, then recurse into the inner definition
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "decorator" {
                    emit_node_full(lines, child.start_position().row, node_last_row(child), output);
                } else if child.is_named() {
                    skeleton_node(child, source, lines, lang, output, depth);
                }
            }
        }
        // Imports: keep as-is
        "import_statement" | "import_from_statement" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        "comment" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        // Expression statements (module-level assignments, etc.)
        "expression_statement" => {
            emit_node_full(lines, start_row, end_row, output);
        }
        _ => {
            emit_node_full(lines, start_row, end_row, output);
        }
    }
}

/// Get the inclusive last row of a node.
/// Tree-sitter's end_position can point to column 0 of the next row (past the newline),
/// so we adjust when that happens.
fn node_last_row(node: Node) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

/// Emit all lines of a node as-is.
fn emit_node_full(lines: &[&str], start_row: usize, end_row: usize, output: &mut String) {
    // end_row is inclusive — but callers may pass tree-sitter end_position().row
    // which can point past the node when column is 0.
    // We cap at lines.len()-1 to be safe.
    let end = (end_row + 1).min(lines.len());
    for line in &lines[start_row..end] {
        output.push_str(line);
        output.push('\n');
    }
}

/// Emit a function/method signature up to the body delimiter, then a placeholder.
/// For `{`-based languages, finds the opening brace and replaces the body with `{ ... }`.
/// For `=>`, finds the arrow and replaces the body with `=> { ... }`.
fn emit_signature_then_body_placeholder(
    node: Node,
    source: &str,
    lines: &[&str],
    output: &mut String,
    delimiter: &str,
    placeholder: &str,
) {
    let start_row = node.start_position().row;
    let end_row = node_last_row(node);

    // For arrow functions with delimiter "=>", find the arrow
    if delimiter == "=>" {
        // Find the "=>" in the node text
        let node_text = node.utf8_text(source.as_bytes()).unwrap_or("");
        if let Some(arrow_offset) = node_text.find("=>") {
            let abs_byte = node.start_byte() + arrow_offset;
            // Find which line the => is on
            let arrow_line = source[..abs_byte].lines().count().saturating_sub(1);
            // Emit lines from start up to the arrow line
            for line in &lines[start_row..arrow_line] {
                output.push_str(line);
                output.push('\n');
            }
            // On the arrow line, emit up to and including '=>' then placeholder
            let line_text = lines[arrow_line];
            // Find => position within this line
            if let Some(pos) = line_text.find("=>") {
                output.push_str(&line_text[..pos]);
                output.push_str(placeholder);
                output.push('\n');
            } else {
                output.push_str(line_text);
                output.push(' ');
                output.push_str(placeholder);
                output.push('\n');
            }
            return;
        }
    }

    // For '{' delimiter: find the opening brace in the body
    if delimiter == "{" {
        // Find the body child (statement_block, block, etc.)
        let body = find_body_child(node);
        if let Some(body_node) = body {
            let brace_row = body_node.start_position().row;
            // Emit lines from start up to (but not including) the brace line
            for line in &lines[start_row..brace_row] {
                output.push_str(line);
                output.push('\n');
            }
            // On the brace line, find the '{' and emit up to it + placeholder
            let line_text = lines[brace_row];
            if let Some(pos) = line_text.find('{') {
                let before_brace = line_text[..pos].trim_end();
                if before_brace.is_empty() && brace_row > start_row {
                    // Brace is on its own line — append to previous line
                    // Remove the last newline we added
                    if output.ends_with('\n') {
                        output.pop();
                    }
                    output.push_str(" { ... }\n");
                } else {
                    output.push_str(before_brace);
                    if !before_brace.is_empty() {
                        output.push(' ');
                    }
                    output.push_str("{ ... }\n");
                }
            } else {
                output.push_str(line_text);
                output.push('\n');
            }
            return;
        }
    }

    // Fallback: emit full node
    emit_node_full(lines, start_row, end_row, output);
}

/// Emit a Python function def as skeleton: `def name(params):` + `    ...`
fn emit_python_def_skeleton(node: Node, lines: &[&str], output: &mut String) {
    let start_row = node.start_position().row;

    // Find the body child
    let body = node.child_by_field_name("body");
    if let Some(body_node) = body {
        let body_start = body_node.start_position().row;
        // Emit the def line(s) up to the body
        for line in &lines[start_row..body_start] {
            output.push_str(line);
            output.push('\n');
        }
        // Add indented ...
        let indent = get_indent(lines, body_start);
        output.push_str(&indent);
        output.push_str("...\n");
    } else {
        // No body found, emit as-is
        let end_row = node_last_row(node);
        emit_node_full(lines, start_row, end_row, output);
    }
}

/// Get the indentation of a line.
fn get_indent(lines: &[&str], row: usize) -> String {
    if row < lines.len() {
        let line = lines[row];
        let trimmed = line.trim_start();
        line[..line.len() - trimmed.len()].to_string()
    } else {
        "    ".to_string()
    }
}

/// Find the body/block child of a node (the part we want to collapse).
fn find_body_child(node: Node) -> Option<Node> {
    // Try common body field names
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }

    // Look for statement_block, block, declaration_list children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "statement_block" | "block" | "declaration_list" | "field_declaration_list" => {
                return Some(child);
            }
            _ => {}
        }
    }
    None
}

/// Get the first line of a node's source text.
/// Uses byte offset for O(1) lookup instead of iterating lines from the start.
fn first_line(node: Node, source: &str) -> String {
    let byte_start = node.start_byte();
    let remaining = &source[byte_start..];
    remaining
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

// ──────────────────────────────────────────────────────────────────────────────
// Reference classification for `ctx flow`
// ──────────────────────────────────────────────────────────────────────────────

/// How a symbol is used at a specific AST location.
/// Variant order defines the narrative flow in output: imports → definitions → usage → output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefKind {
    Import,
    Definition,
    TypeDefinition,
    Export,
    PropReceive,
    Call,
    PropPass,
    ConditionalUse,
    ReturnValue,
    Mutation,
    Read,
}

impl RefKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Import => "IMPORT",
            Self::Definition => "DEFINITION",
            Self::TypeDefinition => "TYPE",
            Self::Export => "EXPORT",
            Self::PropReceive => "PROP RECEIVE",
            Self::Call => "CALL",
            Self::PropPass => "PROP PASS",
            Self::ConditionalUse => "CONDITIONAL",
            Self::ReturnValue => "RETURN",
            Self::Mutation => "MUTATION",
            Self::Read => "READ",
        }
    }
}

impl std::fmt::Display for RefKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Identifier-like node kinds across all supported languages.
const IDENT_KINDS: &[&str] = &[
    // TS/TSX/JS
    "identifier",
    "property_identifier",
    "shorthand_property_identifier",
    "shorthand_property_identifier_pattern",
    "type_identifier",
    // Rust
    "field_identifier",
    // Python uses "identifier" (already listed)
];

/// Find all identifier nodes matching `symbol` on a given line (0-indexed).
pub fn find_identifiers_on_line<'a>(
    tree: &'a Tree,
    source: &str,
    line: usize,
    symbol: &str,
) -> Vec<Node<'a>> {
    let mut results = Vec::new();
    find_idents_recursive(tree.root_node(), source, line, symbol, &mut results);
    results
}

fn find_idents_recursive<'a>(
    node: Node<'a>,
    source: &str,
    target_line: usize,
    symbol: &str,
    results: &mut Vec<Node<'a>>,
) {
    let start_row = node.start_position().row;
    let end_row = node.end_position().row;

    // Skip subtrees that can't contain the target line
    if target_line < start_row || target_line > end_row {
        return;
    }

    // Check if this is a matching identifier on the target line
    if start_row == target_line && IDENT_KINDS.contains(&node.kind()) {
        if node.utf8_text(source.as_bytes()).ok() == Some(symbol) {
            results.push(node);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_idents_recursive(child, source, target_line, symbol, results);
    }
}

/// Classify how an identifier node is used by walking up the AST.
pub fn classify_identifier(ident: Node, source: &str, lang: Lang) -> RefKind {
    match lang {
        Lang::TypeScript | Lang::Tsx | Lang::JavaScript => classify_ident_ts(ident, source),
        Lang::Rust => classify_ident_rust(ident),
        Lang::Python => classify_ident_python(ident),
    }
}

/// Check if `target` is within `container`'s byte range.
fn node_contains_node(container: Node, target: Node) -> bool {
    container.start_byte() <= target.start_byte() && target.end_byte() <= container.end_byte()
}

/// Check if `target` is within the named field of `parent`.
fn is_in_field(parent: Node, field: &str, target: Node) -> bool {
    parent
        .child_by_field_name(field)
        .map(|n| node_contains_node(n, target))
        .unwrap_or(false)
}

fn classify_ident_ts(ident: Node, _source: &str) -> RefKind {
    let mut current = ident;

    for _ in 0..20 {
        let parent = match current.parent() {
            Some(p) => p,
            None => break,
        };

        match parent.kind() {
            // ── Imports ──
            "import_specifier" | "import_clause" | "import_statement" | "namespace_import" => {
                return RefKind::Import;
            }

            // ── Exports ──
            "export_specifier" | "export_clause" => return RefKind::Export,
            "export_statement" => {
                // `export default foo` → direct child of export_statement
                if current.id() == ident.id() {
                    return RefKind::Export;
                }
                // Inside a declaration within export (export const foo = bar) → keep walking
                // will have been caught by variable_declarator/function_declaration etc.
            }

            // ── Declarations ──
            "variable_declarator" => {
                if is_in_field(parent, "name", ident) {
                    return RefKind::Definition;
                }
            }
            "function_declaration" | "generator_function_declaration" | "method_definition"
            | "class_declaration" | "abstract_class_declaration" => {
                if is_in_field(parent, "name", ident) {
                    return RefKind::Definition;
                }
            }

            // ── Calls ──
            "call_expression" | "new_expression" => {
                if is_in_field(parent, "function", ident) {
                    return RefKind::Call;
                }
            }

            // ── JSX ──
            "jsx_attribute" => return RefKind::PropPass,
            "jsx_opening_element" | "jsx_self_closing_element" => {
                // <OurSymbol /> — component usage is like a call
                if is_in_field(parent, "name", ident) {
                    return RefKind::Call;
                }
            }

            // ── Parameters / Destructuring ──
            "required_parameter" | "optional_parameter" => {
                return RefKind::PropReceive;
            }
            "object_pattern" => {
                if let Some(gp) = parent.parent() {
                    match gp.kind() {
                        "required_parameter" | "optional_parameter" => {
                            return RefKind::PropReceive;
                        }
                        "variable_declarator" if is_in_field(gp, "name", parent) => {
                            return RefKind::Definition;
                        }
                        _ => {}
                    }
                }
            }
            "array_pattern" => {
                if let Some(gp) = parent.parent() {
                    if gp.kind() == "variable_declarator" && is_in_field(gp, "name", parent) {
                        return RefKind::Definition;
                    }
                }
            }

            // ── Types ──
            "property_signature" | "method_signature" => return RefKind::TypeDefinition,
            "type_alias_declaration" | "interface_declaration" | "enum_declaration" => {
                if is_in_field(parent, "name", ident) {
                    return RefKind::TypeDefinition;
                }
                return RefKind::TypeDefinition;
            }
            "type_annotation" | "type_arguments" | "constraint" => {
                return RefKind::TypeDefinition;
            }

            // ── Mutation ──
            "assignment_expression" | "augmented_assignment_expression" => {
                if is_in_field(parent, "left", ident) {
                    return RefKind::Mutation;
                }
            }
            "update_expression" => return RefKind::Mutation,

            // ── Control flow ──
            "return_statement" => return RefKind::ReturnValue,
            "if_statement" | "while_statement" | "do_statement" | "switch_statement" => {
                if is_in_field(parent, "condition", ident) {
                    return RefKind::ConditionalUse;
                }
            }
            "ternary_expression" => {
                if is_in_field(parent, "condition", ident) {
                    return RefKind::ConditionalUse;
                }
            }

            _ => {}
        }

        current = parent;
    }

    RefKind::Read
}

fn classify_ident_rust(ident: Node) -> RefKind {
    let mut current = ident;

    for _ in 0..20 {
        let parent = match current.parent() {
            Some(p) => p,
            None => break,
        };

        match parent.kind() {
            "use_declaration" | "use_as_clause" | "use_list" | "use_wildcard" => {
                return RefKind::Import;
            }
            // scoped_identifier is used both in `use` paths and in expressions
            // (e.g., `ts::foo()`). Only classify as Import if inside a use_declaration.
            "scoped_identifier" => {
                let mut ancestor = parent;
                for _ in 0..10 {
                    match ancestor.parent() {
                        Some(a) if a.kind() == "use_declaration" => return RefKind::Import,
                        Some(a) if a.kind() == "scoped_identifier" => { ancestor = a; }
                        _ => break,
                    }
                }
                // Not inside use — continue walking (call_expression will catch it)
            }
            "function_item" | "const_item" | "static_item" | "let_declaration" => {
                if is_in_field(parent, "name", ident) {
                    return RefKind::Definition;
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "type_item" => {
                if is_in_field(parent, "name", ident) {
                    return RefKind::TypeDefinition;
                }
            }
            "impl_item" => {
                if is_in_field(parent, "type", ident) || is_in_field(parent, "trait", ident) {
                    return RefKind::TypeDefinition;
                }
            }
            "call_expression" => {
                if is_in_field(parent, "function", ident) {
                    return RefKind::Call;
                }
            }
            "macro_invocation" => {
                if is_in_field(parent, "macro", ident) {
                    return RefKind::Call;
                }
            }
            "assignment_expression" | "compound_assignment_expr" => {
                if is_in_field(parent, "left", ident) {
                    return RefKind::Mutation;
                }
            }
            "return_expression" => return RefKind::ReturnValue,
            "if_expression" | "while_expression" => {
                if is_in_field(parent, "condition", ident) {
                    return RefKind::ConditionalUse;
                }
            }
            "parameter" | "self_parameter" => return RefKind::PropReceive,
            _ => {}
        }

        current = parent;
    }

    RefKind::Read
}

fn classify_ident_python(ident: Node) -> RefKind {
    let mut current = ident;

    for _ in 0..20 {
        let parent = match current.parent() {
            Some(p) => p,
            None => break,
        };

        match parent.kind() {
            "import_statement" | "import_from_statement" | "aliased_import" => {
                return RefKind::Import;
            }
            "dotted_name" => {
                // Check if this dotted_name is part of an import
                if let Some(gp) = parent.parent() {
                    if gp.kind() == "import_statement" || gp.kind() == "import_from_statement" {
                        return RefKind::Import;
                    }
                }
            }
            "function_definition" | "class_definition" => {
                if is_in_field(parent, "name", ident) {
                    return RefKind::Definition;
                }
            }
            "assignment" => {
                if is_in_field(parent, "left", ident) {
                    return RefKind::Definition;
                }
            }
            "augmented_assignment" => {
                if is_in_field(parent, "left", ident) {
                    return RefKind::Mutation;
                }
            }
            "call" => {
                if is_in_field(parent, "function", ident) {
                    return RefKind::Call;
                }
            }
            "return_statement" => return RefKind::ReturnValue,
            "if_statement" | "while_statement" | "elif_clause" => {
                if is_in_field(parent, "condition", ident) {
                    return RefKind::ConditionalUse;
                }
            }
            "parameters" | "default_parameter" | "typed_parameter"
            | "typed_default_parameter" => {
                return RefKind::PropReceive;
            }
            _ => {}
        }

        current = parent;
    }

    RefKind::Read
}
