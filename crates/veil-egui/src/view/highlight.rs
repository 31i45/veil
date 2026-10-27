//! 轻量代码语法高亮：自研逐行状态机扫描，不引入 syntect。
//!
//! 设计目标：
//! - 覆盖 16 种常用语言的关键字 / 字符串 / 注释 / 数字 / 函数名 / 类型名着色。
//! - 纯函数、零分配关键字表（`&[&str]` 静态切片）。
//! - 输出致密 `(start, end, color)` 段，与 `style_spans` 同形，便于 layout 层直接铺底。
//! - 无法识别的语言走通用扫描（字符串 / 注释 / 数字仍着色）。

/// 高亮 token 对应的语义色（由 Theme.syntax 映射到具体 RGB）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Number,
    Function,
    TypeName,
    Punctuation,
}

/// 单语言配置：行注释前缀、块注释对、字符串引号、关键字表、类型识别策略。
struct LangDef {
    line_comment: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    strings: &'static [char],
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    /// 首字母大写即视为类型（C++/Java/Rust/Go 的命名约定）。
    cap_is_type: bool,
    /// 关键字大小写不敏感（SQL 等）。
    case_insensitive: bool,
}

fn lang_def(lang: &str) -> LangDef {
    match lang {
        "rust" => LangDef {
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\''],
            keywords: &[
                "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false",
                "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
                "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
                "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
                "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof",
                "unsized", "virtual", "yield", "try",
            ],
            types: &[
                "u8", "u16", "u32", "u64", "u128", "i8", "i16", "i32", "i64", "i128", "f32", "f64",
                "bool", "char", "str", "String", "Vec", "Option", "Result", "usize", "isize",
                "Box", "Rc", "Arc", "Cell", "RefCell", "HashMap", "HashSet", "BTreeMap",
                "BTreeSet", "Self",
            ],
            cap_is_type: true,
            case_insensitive: false,
        },
        "python" | "py" => LangDef {
            line_comment: &["#"],
            block_comment: None,
            strings: &['"', '\''],
            keywords: &[
                "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
                "continue", "def", "del", "elif", "else", "except", "finally", "for", "from",
                "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass",
                "raise", "return", "try", "while", "with", "yield", "match", "case",
            ],
            types: &[
                "int",
                "float",
                "str",
                "bool",
                "list",
                "dict",
                "tuple",
                "set",
                "bytes",
                "bytearray",
                "complex",
                "frozenset",
                "range",
                "type",
                "object",
                "Exception",
                "ValueError",
                "TypeError",
                "KeyError",
                "IndexError",
                "AttributeError",
                "RuntimeError",
                "NotImplementedError",
                "StopIteration",
                "GeneratorExit",
                "SystemExit",
                "KeyboardInterrupt",
            ],
            cap_is_type: false,
            case_insensitive: false,
        },
        "javascript" | "js" | "jsx" | "typescript" | "ts" | "tsx" => LangDef {
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\'', '`'],
            keywords: &[
                "abstract",
                "arguments",
                "await",
                "boolean",
                "break",
                "byte",
                "case",
                "catch",
                "char",
                "class",
                "const",
                "continue",
                "debugger",
                "default",
                "delete",
                "do",
                "double",
                "else",
                "enum",
                "eval",
                "export",
                "extends",
                "false",
                "final",
                "finally",
                "float",
                "for",
                "function",
                "goto",
                "if",
                "implements",
                "import",
                "in",
                "instanceof",
                "int",
                "interface",
                "let",
                "long",
                "native",
                "new",
                "null",
                "package",
                "private",
                "protected",
                "public",
                "return",
                "short",
                "static",
                "super",
                "switch",
                "synchronized",
                "this",
                "throw",
                "throws",
                "transient",
                "true",
                "try",
                "typeof",
                "var",
                "void",
                "volatile",
                "while",
                "with",
                "yield",
                "async",
                "of",
                "from",
                "as",
            ],
            types: &[
                "Array",
                "Boolean",
                "Date",
                "Error",
                "Function",
                "JSON",
                "Map",
                "Math",
                "Number",
                "Object",
                "Promise",
                "Proxy",
                "Reflect",
                "RegExp",
                "Set",
                "String",
                "Symbol",
                "WeakMap",
                "WeakSet",
                "console",
                "document",
                "window",
                "globalThis",
            ],
            cap_is_type: false,
            case_insensitive: false,
        },
        "c" | "cpp" | "c++" | "h" | "hpp" | "cc" => LangDef {
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\''],
            keywords: &[
                "alignas",
                "alignof",
                "and",
                "and_eq",
                "asm",
                "auto",
                "bitand",
                "bitor",
                "bool",
                "break",
                "case",
                "catch",
                "char",
                "char8_t",
                "char16_t",
                "char32_t",
                "class",
                "compl",
                "const",
                "consteval",
                "constexpr",
                "constinit",
                "const_cast",
                "continue",
                "decltype",
                "default",
                "delete",
                "do",
                "double",
                "dynamic_cast",
                "else",
                "enum",
                "explicit",
                "export",
                "extern",
                "false",
                "float",
                "for",
                "friend",
                "goto",
                "if",
                "inline",
                "int",
                "long",
                "mutable",
                "namespace",
                "new",
                "noexcept",
                "not",
                "not_eq",
                "nullptr",
                "operator",
                "or",
                "or_eq",
                "private",
                "protected",
                "public",
                "register",
                "reinterpret_cast",
                "return",
                "short",
                "signed",
                "sizeof",
                "static",
                "static_assert",
                "static_cast",
                "struct",
                "switch",
                "template",
                "this",
                "thread_local",
                "throw",
                "true",
                "try",
                "typedef",
                "typeid",
                "typename",
                "union",
                "unsigned",
                "using",
                "virtual",
                "void",
                "volatile",
                "wchar_t",
                "while",
                "xor",
                "xor_eq",
            ],
            types: &[
                "int",
                "char",
                "float",
                "double",
                "bool",
                "void",
                "short",
                "long",
                "unsigned",
                "signed",
                "size_t",
                "ptrdiff_t",
                "int8_t",
                "int16_t",
                "int32_t",
                "int64_t",
                "uint8_t",
                "uint16_t",
                "uint32_t",
                "uint64_t",
                "string",
                "vector",
                "map",
                "set",
                "unordered_map",
                "unordered_set",
                "array",
                "deque",
                "queue",
                "stack",
                "list",
                "forward_list",
                "optional",
                "variant",
                "tuple",
                "pair",
                "shared_ptr",
                "unique_ptr",
                "weak_ptr",
                "std",
                "cout",
                "cin",
                "cerr",
                "endl",
            ],
            cap_is_type: true,
            case_insensitive: false,
        },
        "java" => LangDef {
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\''],
            keywords: &[
                "abstract",
                "assert",
                "boolean",
                "break",
                "byte",
                "case",
                "catch",
                "char",
                "class",
                "const",
                "continue",
                "default",
                "do",
                "double",
                "else",
                "enum",
                "extends",
                "final",
                "finally",
                "float",
                "for",
                "goto",
                "if",
                "implements",
                "import",
                "instanceof",
                "int",
                "interface",
                "long",
                "native",
                "new",
                "package",
                "private",
                "protected",
                "public",
                "return",
                "short",
                "static",
                "strictfp",
                "super",
                "switch",
                "synchronized",
                "this",
                "throw",
                "throws",
                "transient",
                "try",
                "void",
                "volatile",
                "while",
                "true",
                "false",
                "null",
                "var",
                "yield",
                "record",
                "sealed",
                "permits",
                "non-sealed",
            ],
            types: &[
                "String",
                "Integer",
                "Long",
                "Double",
                "Float",
                "Boolean",
                "Character",
                "Byte",
                "Short",
                "Object",
                "Class",
                "System",
                "Math",
                "Arrays",
                "Collections",
                "List",
                "Map",
                "Set",
                "ArrayList",
                "HashMap",
                "HashSet",
                "LinkedList",
                "TreeMap",
                "TreeSet",
                "Optional",
                "Stream",
                "Exception",
                "RuntimeException",
                "IOException",
                "NullPointerException",
                "IllegalArgumentException",
                "IndexOutOfBoundsException",
            ],
            cap_is_type: true,
            case_insensitive: false,
        },
        "go" | "golang" => LangDef {
            line_comment: &["//"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\'', '`'],
            keywords: &[
                "break",
                "case",
                "chan",
                "const",
                "continue",
                "default",
                "defer",
                "else",
                "fallthrough",
                "for",
                "func",
                "go",
                "goto",
                "if",
                "import",
                "interface",
                "map",
                "package",
                "range",
                "return",
                "select",
                "struct",
                "switch",
                "type",
                "var",
                "true",
                "false",
                "nil",
            ],
            types: &[
                "int",
                "int8",
                "int16",
                "int32",
                "int64",
                "uint",
                "uint8",
                "uint16",
                "uint32",
                "uint64",
                "uintptr",
                "float32",
                "float64",
                "complex64",
                "complex128",
                "bool",
                "byte",
                "rune",
                "string",
                "error",
                "any",
                "struct",
                "interface",
                "map",
                "chan",
                "Context",
                "Reader",
                "Writer",
                "Closer",
                "ReadCloser",
                "WriteCloser",
            ],
            cap_is_type: true,
            case_insensitive: false,
        },
        "json" => LangDef {
            line_comment: &[],
            block_comment: None,
            strings: &['"'],
            keywords: &["true", "false", "null"],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        "yaml" | "yml" => LangDef {
            line_comment: &["#"],
            block_comment: None,
            strings: &['"', '\''],
            keywords: &["true", "false", "null", "yes", "no", "on", "off", "~"],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        "toml" => LangDef {
            line_comment: &["#"],
            block_comment: None,
            strings: &['"', '\''],
            keywords: &["true", "false"],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        "bash" | "sh" | "shell" | "zsh" => LangDef {
            line_comment: &["#"],
            block_comment: None,
            strings: &['"', '\''],
            keywords: &[
                "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "case", "esac",
                "function", "return", "exit", "export", "local", "declare", "readonly", "unset",
                "source", "eval", "exec", "break", "continue", "in", "select", "until", "set",
                "unset", "shift", "cd", "echo", "printf", "test", "[", "]", "[[", "]]",
            ],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        "sql" => LangDef {
            line_comment: &["--", "#"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\''],
            keywords: &[
                "SELECT",
                "FROM",
                "WHERE",
                "INSERT",
                "INTO",
                "VALUES",
                "UPDATE",
                "SET",
                "DELETE",
                "CREATE",
                "TABLE",
                "DROP",
                "ALTER",
                "ADD",
                "COLUMN",
                "INDEX",
                "VIEW",
                "PRIMARY",
                "KEY",
                "FOREIGN",
                "REFERENCES",
                "JOIN",
                "INNER",
                "LEFT",
                "RIGHT",
                "OUTER",
                "FULL",
                "ON",
                "GROUP",
                "BY",
                "ORDER",
                "HAVING",
                "LIMIT",
                "OFFSET",
                "UNION",
                "INTERSECT",
                "EXCEPT",
                "AS",
                "AND",
                "OR",
                "NOT",
                "IN",
                "EXISTS",
                "BETWEEN",
                "LIKE",
                "IS",
                "NULL",
                "TRUE",
                "FALSE",
                "CASE",
                "WHEN",
                "THEN",
                "ELSE",
                "END",
                "DISTINCT",
                "ALL",
                "ANY",
                "SOME",
                "ASC",
                "DESC",
                "DEFAULT",
                "CHECK",
                "CONSTRAINT",
                "UNIQUE",
                "AUTO_INCREMENT",
                "CASCADE",
                "TRIGGER",
                "PROCEDURE",
                "FUNCTION",
                "RETURNS",
                "BEGIN",
                "COMMIT",
                "ROLLBACK",
                "GRANT",
                "REVOKE",
                "SCHEMA",
                "DATABASE",
                "USE",
                "SHOW",
                "DESCRIBE",
                "EXPLAIN",
            ],
            types: &[
                "INT",
                "INTEGER",
                "BIGINT",
                "SMALLINT",
                "TINYINT",
                "VARCHAR",
                "CHAR",
                "TEXT",
                "DATE",
                "TIME",
                "DATETIME",
                "TIMESTAMP",
                "FLOAT",
                "DOUBLE",
                "DECIMAL",
                "NUMERIC",
                "BOOLEAN",
                "BOOL",
                "BLOB",
                "JSON",
                "UUID",
                "SERIAL",
                "BINARY",
                "VARBINARY",
                "ENUM",
                "SET",
            ],
            cap_is_type: false,
            case_insensitive: true,
        },
        "html" | "htm" | "xml" => LangDef {
            line_comment: &[],
            block_comment: Some(("<!--", "-->")),
            strings: &['"', '\''],
            keywords: &[],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        "css" | "scss" | "sass" | "less" => LangDef {
            line_comment: &[],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\''],
            keywords: &[
                "important",
                "inherit",
                "initial",
                "unset",
                "auto",
                "none",
                "solid",
                "dashed",
                "dotted",
                "hidden",
                "visible",
                "scroll",
                "relative",
                "absolute",
                "fixed",
                "static",
                "sticky",
                "block",
                "inline",
                "flex",
                "grid",
                "center",
                "left",
                "right",
                "top",
                "bottom",
                "px",
                "em",
                "rem",
                "vw",
                "vh",
                "%",
            ],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        "markdown" | "md" => LangDef {
            line_comment: &[],
            block_comment: Some(("<!--", "-->")),
            strings: &['"', '\''],
            keywords: &[],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
        _ => LangDef {
            line_comment: &["//", "#"],
            block_comment: Some(("/*", "*/")),
            strings: &['"', '\'', '`'],
            keywords: &[],
            types: &[],
            cap_is_type: false,
            case_insensitive: false,
        },
    }
}

/// 扫描 `text` 按 `lang` 着色，返回致密 `(start, end, TokenKind)` 段。
/// 文本中无 token 覆盖的间隙由调用方填默认样式。
pub fn highlight(lang: &str, text: &str) -> Vec<(usize, usize, TokenKind)> {
    let def = lang_def(&lang.to_ascii_lowercase());
    let bytes = text.as_bytes();
    let mut out: Vec<(usize, usize, TokenKind)> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let rem = &text[i..];

        // --- 行注释 ---
        let mut is_line_comment = false;
        for lc in def.line_comment {
            if rem.starts_with(lc) {
                // 到行尾
                let line_end = text[i..].find('\n').map(|p| i + p).unwrap_or(text.len());
                out.push((i, line_end, TokenKind::Comment));
                i = line_end;
                is_line_comment = true;
                break;
            }
        }
        if is_line_comment {
            continue;
        }

        // --- 块注释（一次性扫描到结束标记，可跨多行） ---
        if let Some((start, end)) = def.block_comment {
            if rem.starts_with(start) {
                let after_start = i + start.len();
                let end_pos = text[after_start..]
                    .find(end)
                    .map(|p| after_start + p + end.len())
                    .unwrap_or(text.len());
                out.push((i, end_pos, TokenKind::Comment));
                i = end_pos;
                continue;
            }
        }

        let c = bytes[i] as char;

        // --- 字符串 ---
        if def.strings.contains(&c) {
            let quote = c;
            let start = i;
            i += 1;
            // 处理三引号（Python/JS）
            let triple =
                i + 2 < bytes.len() && bytes[i] == quote as u8 && bytes[i + 1] == quote as u8;
            if triple {
                let end_seq_start = i; // 第二个/第三个引号起点
                let end_seq = format!("{quote}{quote}{quote}");
                let rest = &text[end_seq_start..];
                let end_pos = rest
                    .find(&end_seq)
                    .map(|p| end_seq_start + p + 3)
                    .unwrap_or(text.len());
                out.push((start, end_pos, TokenKind::String));
                i = end_pos;
                continue;
            }
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch == '\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if ch == quote {
                    i += 1;
                    break;
                }
                if ch == '\n' {
                    // 未闭合字符串到行尾
                    break;
                }
                i += 1;
            }
            out.push((start, i, TokenKind::String));
            continue;
        }

        // --- 数字 ---
        if c.is_ascii_digit()
            || (c == '.' && i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_digit())
        {
            let start = i;
            // 跳过 0x / 0o / 0b 前缀
            if c == '0' && i + 1 < bytes.len() {
                let nxt = bytes[i + 1] as char;
                if matches!(nxt, 'x' | 'X' | 'o' | 'O' | 'b' | 'B') {
                    i += 2;
                }
            }
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                    i += 1;
                } else {
                    break;
                }
            }
            // 数字后缀（如 100u32, 1.5f64）
            out.push((start, i, TokenKind::Number));
            continue;
        }

        // --- 标识符 ---
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < bytes.len() {
                let ch = bytes[i] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let word = &text[start..i];
            // 判断下一个非空白字符是否为 '(' → 函数调用
            let mut j = i;
            while j < bytes.len() && (bytes[j] as char).is_whitespace() {
                j += 1;
            }
            let is_func_call = j < bytes.len() && bytes[j] as char == '(';

            let kw_match = if def.case_insensitive {
                let lower = word.to_ascii_lowercase();
                def.keywords.iter().any(|k| k.to_ascii_lowercase() == lower)
            } else {
                def.keywords.contains(&word)
            };
            let type_match = if def.case_insensitive {
                let lower = word.to_ascii_lowercase();
                def.types.iter().any(|t| t.to_ascii_lowercase() == lower)
            } else {
                def.types.contains(&word)
            };
            let kind = if kw_match {
                TokenKind::Keyword
            } else if type_match {
                TokenKind::TypeName
            } else if def.cap_is_type
                && word
                    .chars()
                    .next()
                    .map(|f| f.is_ascii_uppercase())
                    .unwrap_or(false)
            {
                // 全大写（如常量）不当作类型
                let is_all_upper = word
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
                if is_all_upper {
                    TokenKind::Keyword
                } else {
                    TokenKind::TypeName
                }
            } else if is_func_call {
                TokenKind::Function
            } else {
                out.push((start, i, TokenKind::Punctuation));
                // plain：不产出彩色段，让默认样式覆盖
                // 但我们需要保持致密输出——这里 push 一个 Punctuation 占位不对，
                // plain 文本不产出段，由 layout 层填默认色。
                out.pop();
                continue;
            };
            out.push((start, i, kind));
            continue;
        }

        // 其余字符跳过（默认色）
        i += 1;
    }

    // 合并相邻同类型段（减少 layout 层 span 数）
    merge_adjacent(&mut out);
    out
}

fn merge_adjacent(spans: &mut Vec<(usize, usize, TokenKind)>) {
    if spans.len() < 2 {
        return;
    }
    let mut write = 0;
    for read in 1..spans.len() {
        if spans[read].2 == spans[write].2 && spans[read].0 == spans[write].1 {
            spans[write].1 = spans[read].1;
        } else {
            write += 1;
            spans[write] = spans[read];
        }
    }
    spans.truncate(write + 1);
}

/// 把高亮段（稀疏彩色 token）补全为致密 `(start, end, Style)`，
/// 间隙用 `default` 填充，彩色段用对应 `color`。
pub fn to_styles(
    spans: &[(usize, usize, TokenKind)],
    text_len: usize,
    colors: &crate::theme::SyntaxColors,
    default: Style,
) -> Vec<(usize, usize, Style)> {
    let mut out: Vec<(usize, usize, Style)> = Vec::new();
    let mut pos = 0;
    for &(s, e, tk) in spans {
        if s > pos {
            out.push((pos, s, default));
        }
        let color = match tk {
            TokenKind::Keyword => colors.keyword,
            TokenKind::String => colors.string,
            TokenKind::Comment => colors.comment,
            TokenKind::Number => colors.number,
            TokenKind::Function => colors.function,
            TokenKind::TypeName => colors.type_name,
            TokenKind::Punctuation => colors.punctuation,
        };
        let mut st = default;
        st.color = Some(color);
        out.push((s, e, st));
        pos = e;
    }
    if pos < text_len {
        out.push((pos, text_len, default));
    }
    out
}

use crate::view::style::Style;

#[cfg(test)]
mod tests {
    use super::*;

    fn color_at(spans: &[(usize, usize, TokenKind)], at: usize) -> Option<TokenKind> {
        spans
            .iter()
            .find(|(s, e, _)| *s <= at && at < *e)
            .map(|x| x.2)
    }

    #[test]
    fn rust_keyword_string_comment() {
        let src = "// hi\nlet x = \"a\";\nfn f() {}";
        let sp = highlight("rust", src);
        assert_eq!(color_at(&sp, 0), Some(TokenKind::Comment)); // // hi
        assert_eq!(color_at(&sp, 6), Some(TokenKind::Keyword)); // let
        assert_eq!(color_at(&sp, 14), Some(TokenKind::String)); // "a"
        assert_eq!(color_at(&sp, 19), Some(TokenKind::Keyword)); // fn
        assert_eq!(color_at(&sp, 22), Some(TokenKind::Function)); // f
    }

    #[test]
    fn python_number_and_def() {
        let sp = highlight("python", "def foo(n):\n    return n + 42\n");
        assert_eq!(color_at(&sp, 0), Some(TokenKind::Keyword)); // def
        assert_eq!(color_at(&sp, 4), Some(TokenKind::Function)); // foo
        assert_eq!(color_at(&sp, 27), Some(TokenKind::Number)); // 42
    }

    #[test]
    fn block_comment_spans_lines() {
        let sp = highlight("rust", "a /* multi\nline */ b");
        // 整个块注释为 Comment
        assert_eq!(color_at(&sp, 2), Some(TokenKind::Comment));
        assert_eq!(color_at(&sp, 10), Some(TokenKind::Comment));
        assert_eq!(color_at(&sp, 18), None); // b
    }

    #[test]
    fn triple_quoted_string() {
        let sp = highlight("python", "x = \"\"\"multi\nline\"\"\"");
        assert_eq!(color_at(&sp, 4), Some(TokenKind::String));
        assert_eq!(color_at(&sp, 12), Some(TokenKind::String));
    }

    #[test]
    fn uppercase_const_is_keyword() {
        let sp = highlight("rust", "const MAX: u32 = 100;");
        assert_eq!(color_at(&sp, 6), Some(TokenKind::Keyword)); // MAX
    }

    #[test]
    fn type_camel_case() {
        let sp = highlight("rust", "let v: Vec<String> = Vec::new();");
        assert_eq!(color_at(&sp, 8), Some(TokenKind::TypeName)); // Vec
    }

    #[test]
    fn json_keys_and_values() {
        let sp = highlight("json", "{\"a\": true, \"b\": 1}");
        assert_eq!(color_at(&sp, 1), Some(TokenKind::String)); // "a"
        assert_eq!(color_at(&sp, 7), Some(TokenKind::Keyword)); // true
    }

    #[test]
    fn unknown_lang_falls_back() {
        let sp = highlight("brainfuck", "// comment\n\"str\" 42");
        assert_eq!(color_at(&sp, 0), Some(TokenKind::Comment));
        assert_eq!(color_at(&sp, 12), Some(TokenKind::String));
        assert_eq!(color_at(&sp, 18), Some(TokenKind::Number));
    }

    #[test]
    fn dense_coverage_from_to_styles() {
        use crate::theme::Theme;
        let sp = highlight("rust", "let x = 1;");
        let styles = to_styles(&sp, 10, &Theme::light().syntax, Style::default());
        let mut pos = 0;
        for &(s, e, _) in &styles {
            assert_eq!(s, pos, "致密覆盖无间隙");
            pos = e;
        }
        assert_eq!(pos, 10);
    }
}
