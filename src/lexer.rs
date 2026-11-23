#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Word,
    Number(i32),
    String(String),
    Symbol,
    End,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    pub line: usize,
    pub column: usize,
    pub kind: Kind,
}
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOKENS: usize = 1_000_000;
pub fn lex(source: &str) -> Result<Vec<Token>, String> {
    lex_with_token_limit(source, MAX_TOKENS)
}
/// Smaller budgets may be selected by callers; the global ceiling still applies.
pub fn lex_with_token_limit(source: &str, max_tokens: usize) -> Result<Vec<Token>, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("1:1: source input exceeds16MiB budget".into());
    }
    let max_tokens = max_tokens.min(MAX_TOKENS);
    let chars: Vec<char> = source.chars().collect();
    let (mut i, mut line, mut column) = (0, 1, 1);
    let mut tokens = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
            i += 1;
            continue;
        }
        if c == '#' && column == 1 {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
                column += 1;
            }
            let directive: String = chars[start..i].iter().collect();
            if !tokens.is_empty() || directive.trim() != "#include <stdio.h>" {
                return Err(format!("{line}:1: unsupported preprocessor directive"));
            }
            continue;
        }
        if i + 1 < chars.len()
            && matches!(
                (c, chars[i + 1]),
                ('+', '+')
                    | ('-', '-')
                    | ('+', '=')
                    | ('-', '=')
                    | ('*', '=')
                    | ('/', '=')
                    | ('%', '=')
                    | ('<', '<')
                    | ('>', '>')
            )
        {
            return Err(format!("{line}:{column}: unsupported compound operator"));
        }
        let (start, col) = (i, column);
        let kind = if c.is_ascii_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            Kind::Word
        } else if c.is_ascii_digit() {
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let value: String = chars[start..i].iter().collect();
            if value.len() > 1 && value.starts_with('0') {
                return Err(format!("{line}:{col}: nondecimal literals are unsupported"));
            }
            Kind::Number(value.parse().map_err(|_| {
                format!("{line}:{col}: integer literal exceeds signed 32-bit range")
            })?)
        } else if c == '"' {
            i += 1;
            let mut value = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\0' {
                    return Err(format!(
                        "{line}:{col}: raw NUL is unsupported in source strings"
                    ));
                }
                if chars[i] == '\n' || chars[i] == '\r' {
                    return Err(format!("{line}:{col}: unterminated string"));
                }
                if chars[i] == '\\' {
                    i += 1;
                    value.push(match chars.get(i) {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('"') => '"',
                        Some('\\') => '\\',
                        _ => return Err(format!("{line}:{col}: unsupported string escape")),
                    });
                } else {
                    value.push(chars[i]);
                }
                i += 1;
            }
            if i == chars.len() {
                return Err(format!("{line}:{col}: unterminated string"));
            }
            i += 1;
            Kind::String(value)
        } else if "{}();,+-*/%<>=!&|".contains(c) {
            i += 1;
            if i < chars.len()
                && matches!(
                    (c, chars[i]),
                    ('<', '=') | ('>', '=') | ('=', '=') | ('!', '=') | ('&', '&') | ('|', '|')
                )
            {
                i += 1;
            } else if c == '&' || c == '|' {
                return Err(format!("{line}:{col}: unsupported operator {c}"));
            }
            Kind::Symbol
        } else {
            return Err(format!("{line}:{col}: invalid character {c:?}"));
        };
        if tokens.len() >= max_tokens {
            return Err(format!("{line}:{col}: token budget exceeded"));
        }
        tokens.push(Token {
            text: chars[start..i].iter().collect(),
            line,
            column: col,
            kind,
        });
        column += i - start;
    }
    tokens.push(Token {
        text: String::new(),
        line,
        column,
        kind: Kind::End,
    });
    Ok(tokens)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions() {
        let t = lex("int main() {\n  int value = 12 + 3;\n}").unwrap();
        assert_eq!((t[5].text.as_str(), t[5].line, t[5].column), ("int", 2, 3));
        assert_eq!(t[8].kind, Kind::Number(12));
    }
    #[test]
    fn operators() {
        let t = lex("<= >= == != && || ! + - * / %").unwrap();
        assert_eq!(t.len(), 13);
        assert_eq!(t[0].text, "<=");
    }
    #[test]
    fn strings() {
        assert_eq!(
            lex("\"a\\n\\t\"").unwrap()[0].kind,
            Kind::String("a\n\t".into())
        );
    }
    #[test]
    fn invalid() {
        for s in [
            "@",
            "&",
            "|",
            "2147483648",
            "\"unfinished",
            "\"a\nb\"",
            "\"\\q\"",
            "#define X 1",
        ] {
            assert!(lex(s).is_err(), "{s}");
        }
        assert!(lex("\n @").unwrap_err().starts_with("2:2:"));
    }
    #[test]
    fn header() {
        assert_eq!(lex("#include <stdio.h>\nint").unwrap()[0].line, 2);
        assert_eq!(lex("").unwrap().len(), 1);
    }
}
