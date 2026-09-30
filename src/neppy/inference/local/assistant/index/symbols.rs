//! Regex symbol extraction. Deliberately shallow: a declaration line, its
//! name and its kind, for the languages this project is written in. It is a
//! ranking signal for retrieval, not a parser.

use once_cell::sync::Lazy;
use regex::Regex;

/// Symbols kept per file, so one generated-looking file cannot dominate the
/// table.
pub const MAX_SYMBOLS_PER_FILE: usize = 2000;
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Script,
    Python,
    Markdown,
    Other,
}

impl Lang {
    pub fn from_path(path: &str) -> Self {
        let ext = path
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "rs" => Self::Rust,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Self::Script,
            "py" => Self::Python,
            "md" | "mdx" => Self::Markdown,
            _ => Self::Other,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Script => "script",
            Self::Python => "python",
            Self::Markdown => "markdown",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    /// 1-based.
    pub line: u32,
}

static RUST_ITEM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe|default)\s+)*(?:extern\s+"[^"]*"\s+)?(fn|struct|enum|trait|type|const|static|mod|union)\s+([A-Za-z_][A-Za-z0-9_]*)"#,
    )
    .expect("rust item regex")
});
static RUST_IMPL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^\s*(?:unsafe\s+)?impl(?:<[^>]*>)?\s+(?:[A-Za-z_][A-Za-z0-9_:]*(?:<[^>]*>)?\s+for\s+)?([A-Za-z_][A-Za-z0-9_:]*)",
    )
    .expect("rust impl regex")
});
static RUST_MACRO: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"macro_rules!\s+([A-Za-z_][A-Za-z0-9_]*)").expect("macro regex"));
static SCRIPT_ITEM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^\s*(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(function\*?|class|interface|type|enum)\s+([A-Za-z_$][A-Za-z0-9_$]*)",
    )
    .expect("script item regex")
});
static SCRIPT_CONST: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*[=:]")
        .expect("script const regex")
});
static PY_ITEM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(?:async\s+)?(def|class)\s+([A-Za-z_][A-Za-z0-9_]*)").expect("py regex")
});
static MD_HEADING: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^#{1,6}\s+(.+?)\s*#*\s*$").expect("md regex"));

fn push(out: &mut Vec<Symbol>, name: &str, kind: &str, line: usize) {
    if out.len() >= MAX_SYMBOLS_PER_FILE || name.is_empty() {
        return;
    }
    out.push(Symbol {
        name: name.chars().take(MAX_NAME_CHARS).collect(),
        kind: kind.to_string(),
        line: line as u32,
    });
}

/// Declarations in `text`, in source order.
pub fn extract_symbols(lang: Lang, text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    if lang == Lang::Other {
        return out;
    }
    let mut in_fence = false;
    for (idx, line) in text.lines().enumerate() {
        let number = idx + 1;
        match lang {
            Lang::Rust => {
                if let Some(c) = RUST_ITEM.captures(line) {
                    push(&mut out, &c[2], &c[1], number);
                } else if let Some(c) = RUST_IMPL.captures(line) {
                    push(&mut out, &c[1], "impl", number);
                } else if let Some(c) = RUST_MACRO.captures(line) {
                    push(&mut out, &c[1], "macro", number);
                }
            }
            Lang::Script => {
                if let Some(c) = SCRIPT_ITEM.captures(line) {
                    let kind = c[1].trim_end_matches('*');
                    push(&mut out, &c[2], kind, number);
                } else if let Some(c) = SCRIPT_CONST.captures(line) {
                    push(&mut out, &c[1], "const", number);
                }
            }
            Lang::Python => {
                if let Some(c) = PY_ITEM.captures(line) {
                    push(&mut out, &c[2], &c[1], number);
                }
            }
            Lang::Markdown => {
                if line.trim_start().starts_with("```") {
                    in_fence = !in_fence;
                } else if !in_fence {
                    if let Some(c) = MD_HEADING.captures(line) {
                        push(&mut out, c[1].trim(), "heading", number);
                    }
                }
            }
            Lang::Other => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(symbols: &[Symbol]) -> Vec<(&str, &str)> {
        symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect()
    }

    #[test]
    fn rust_declarations_are_found_with_their_kind() {
        let src = "pub(crate) async fn foo_bar(x: u8) {}\nstruct Point;\n  pub enum Mode {}\n\
                   impl Display for Point {}\nimpl<T> Holder<T> {}\nmacro_rules! my_mac {}\n\
                   pub const LIMIT: u32 = 1;\nmod inner;\n";
        let got = extract_symbols(Lang::Rust, src);
        assert_eq!(
            names(&got),
            vec![
                ("foo_bar", "fn"),
                ("Point", "struct"),
                ("Mode", "enum"),
                ("Point", "impl"),
                ("Holder", "impl"),
                ("my_mac", "macro"),
                ("LIMIT", "const"),
                ("inner", "mod"),
            ]
        );
        assert_eq!(got[0].line, 1);
        assert_eq!(got[7].line, 8);
    }

    #[test]
    fn script_declarations_are_found() {
        let src = "export default function App() {}\nexport const useThing = () => 1;\n\
                   interface Props {}\nexport type Id = string;\nclass A {}\n";
        let got = extract_symbols(Lang::Script, src);
        assert_eq!(
            names(&got),
            vec![
                ("App", "function"),
                ("useThing", "const"),
                ("Props", "interface"),
                ("Id", "type"),
                ("A", "class"),
            ]
        );
    }

    #[test]
    fn python_and_markdown_declarations_are_found() {
        let py = extract_symbols(Lang::Python, "class A:\n    async def run(self):\n");
        assert_eq!(names(&py), vec![("A", "class"), ("run", "def")]);
        let md = extract_symbols(
            Lang::Markdown,
            "# Title\ntext\n```\n# not a heading\n```\n## Sub ##\n",
        );
        assert_eq!(names(&md), vec![("Title", "heading"), ("Sub", "heading")]);
    }

    #[test]
    fn unknown_languages_yield_nothing_and_the_cap_holds() {
        assert!(extract_symbols(Lang::Other, "fn x() {}").is_empty());
        let big = "fn a() {}\n".repeat(MAX_SYMBOLS_PER_FILE + 50);
        assert_eq!(
            extract_symbols(Lang::Rust, &big).len(),
            MAX_SYMBOLS_PER_FILE
        );
        assert_eq!(Lang::from_path("a/b.TSX"), Lang::Script);
    }
}
