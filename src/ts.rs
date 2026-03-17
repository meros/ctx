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
                "impl_item",
                "trait_item",
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

    /// Node kinds that represent exported/public symbols.
    fn exported_symbol_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => &[
                "export_statement",
            ],
            Lang::Rust => &[
                "function_item",
                "struct_item",
                "enum_item",
                "trait_item",
                "type_item",
                "const_item",
                "static_item",
                "impl_item",
                "mod_item",
            ],
            Lang::Python => &[
                "function_definition",
                "class_definition",
                "decorated_definition",
                "assignment",
            ],
        }
    }
}

/// Parse source code with tree-sitter.
pub fn parse(source: &str, lang: Lang) -> Result<Tree> {
    let mut parser = Parser::new();
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

fn first_line(node: Node, source: &str) -> String {
    let start = node.start_position();
    source
        .lines()
        .nth(start.row)
        .unwrap_or("")
        .trim()
        .to_string()
}
