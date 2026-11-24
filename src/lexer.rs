#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Word,
    Number(i32),
    Integer(IntegerLiteral),
    String(String),
    Symbol,
    End,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    pub line: usize,
    pub column: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub kind: Kind,
}
#[derive(Debug, Clone, PartialEq)]
pub struct IntegerLiteral {
    pub magnitude: u64,
    pub radix: u32,
    pub suffix: String,
}
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOKENS: usize = 1_000_000;
pub fn lex(source: &str) -> Result<Vec<Token>, String> {
    lex_with_token_limit(source, MAX_TOKENS)
}
/// Smaller budgets may be selected by callers; the global ceiling still applies.
pub fn lex_with_token_limit(source: &str, max_tokens: usize) -> Result<Vec<Token>, String> {
    scan(source, max_tokens, false)
}
/// Typed frontend tokenization; no conventional-header shortcut is permitted.
pub fn lex_typed(source: &str) -> Result<Vec<Token>, String> {
    scan(source, MAX_TOKENS, true)
}
fn scan(source: &str, max_tokens: usize, typed: bool) -> Result<Vec<Token>, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("1:1: source input exceeds16MiB budget".into());
    }
    if source.contains('\0') {
        return Err("1:1: raw NUL is invalid in source".into());
    }
    let max_tokens = max_tokens.min(MAX_TOKENS);
    let chars: Vec<char> = source.chars().collect();
    let (mut i, mut line, mut column) = (0, 1, 1);
    let mut tokens = Vec::new();
    let mut byte_cursor = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            byte_cursor += c.len_utf8();
            if c == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
            i += 1;
            continue;
        }
        if c == '#' && typed {
            return Err(format!(
                "{line}:{column}: preprocessing is not yet available in typed mode"
            ));
        }
        if c == '#' && column == 1 {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
                column += 1;
            }
            let directive: String = chars[start..i].iter().collect();
            byte_cursor += directive.len();
            if !tokens.is_empty() || directive.trim() != "#include <stdio.h>" {
                return Err(format!("{line}:1: unsupported preprocessor directive"));
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            let comment_start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
                column += 1;
            }
            byte_cursor += chars[comment_start..i]
                .iter()
                .map(|c| c.len_utf8())
                .sum::<usize>();
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let origin = (line, column);
            let comment_start = i;
            i += 2;
            column += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    line += 1;
                    column = 1;
                } else {
                    column += 1;
                }
                i += 1;
            }
            if i == chars.len() {
                return Err(format!("{}:{}: unterminated comment", origin.0, origin.1));
            }
            i += 2;
            column += 2;
            byte_cursor += chars[comment_start..i]
                .iter()
                .map(|c| c.len_utf8())
                .sum::<usize>();
            continue;
        }
        let (start, col) = (i, column);
        let triple = typed
            && chars.get(i..i + 3).is_some_and(|s| {
                s == ['<', '<', '='] || s == ['>', '>', '='] || s == ['.', '.', '.']
            });
        let kind = if triple {
            i += 3;
            Kind::Symbol
        } else if c.is_ascii_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            Kind::Word
        } else if c.is_ascii_digit() {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let spelling: String = chars[start..i].iter().collect();
            let (radix, prefix) = if spelling.starts_with("0x") || spelling.starts_with("0X") {
                (16, 2)
            } else if spelling.starts_with('0') && spelling.len() > 1 {
                (8, 0)
            } else {
                (10, 0)
            };
            let digits: String = spelling[prefix..]
                .chars()
                .take_while(|c| c.is_digit(radix))
                .collect();
            let suffix = spelling[prefix + digits.len()..].to_string();
            let normalized = suffix.to_ascii_lowercase();
            if digits.is_empty()
                || !["", "u", "l", "ul", "lu", "ll", "ull", "llu"].contains(&normalized.as_str())
                || (suffix.contains('l') && suffix.contains('L'))
            {
                return Err(format!("{line}:{col}: invalid integer literal"));
            }
            let magnitude = u64::from_str_radix(&digits, radix)
                .map_err(|_| format!("{line}:{col}: integer literal exceeds 64-bit range"))?;
            if radix == 10 && suffix.is_empty() && (!typed || magnitude <= i32::MAX as u64) {
                Kind::Number(i32::try_from(magnitude).map_err(|_| {
                    format!("{line}:{col}: integer literal exceeds current signed 32-bit range")
                })?)
            } else {
                Kind::Integer(IntegerLiteral {
                    magnitude,
                    radix,
                    suffix,
                })
            }
        } else if c == '\'' {
            i += 1;
            if chars.get(i) == Some(&'\'') {
                return Err(format!("{line}:{col}: empty character constant"));
            }
            let value =
                literal_character(&chars, &mut i).map_err(|e| format!("{line}:{col}: {e}"))?;
            if chars.get(i) != Some(&'\'') {
                return Err(format!("{line}:{col}: expected one character constant"));
            }
            i += 1;
            if !value.is_ascii() {
                return Err(format!(
                    "{line}:{col}: non-ASCII character constant requires later character typing"
                ));
            }
            Kind::Number(value as i32)
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
                value.push(
                    literal_character(&chars, &mut i).map_err(|e| format!("{line}:{col}: {e}"))?,
                );
            }
            if i == chars.len() {
                return Err(format!("{line}:{col}: unterminated string"));
            }
            i += 1;
            Kind::String(value)
        } else if "{}();,:+-*/%<>=!&|".contains(c) || (typed && "[]?~^.".contains(c)) {
            i += 1;
            let pair = chars.get(i).is_some_and(|next| {
                matches!(
                    (c, *next),
                    ('<', '=')
                        | ('>', '=')
                        | ('=', '=')
                        | ('!', '=')
                        | ('&', '&')
                        | ('|', '|')
                        | ('+', '+')
                        | ('-', '-')
                        | ('+', '=')
                        | ('-', '=')
                        | ('*', '=')
                        | ('/', '=')
                        | ('%', '=')
                ) || (typed
                    && matches!(
                        (c, *next),
                        ('<', '<') | ('>', '>') | ('&', '=') | ('|', '=') | ('^', '=') | ('-', '>')
                    ))
            });
            if pair {
                i += 1;
            } else if !typed && (c == '&' || c == '|') {
                return Err(format!("{line}:{col}: unsupported operator {c}"));
            }
            Kind::Symbol
        } else {
            return Err(format!("{line}:{col}: invalid character {c:?}"));
        };
        if tokens.len() >= max_tokens {
            return Err(format!("{line}:{col}: token budget exceeded"));
        }
        let text: String = chars[start..i].iter().collect();
        let byte_start = byte_cursor;
        byte_cursor += text.len();
        tokens.push(Token {
            text,
            byte_start,
            byte_end: byte_cursor,
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
        byte_start: source.len(),
        byte_end: source.len(),
    });
    Ok(tokens)
}

fn literal_character(chars: &[char], i: &mut usize) -> Result<char, String> {
    let c = *chars.get(*i).ok_or("unterminated literal")?;
    if c == '\n' || c == '\r' || c == '\0' {
        return Err("invalid literal character".into());
    }
    *i += 1;
    if c != '\\' {
        return Ok(c);
    }
    let escape = *chars.get(*i).ok_or("unterminated escape")?;
    *i += 1;
    Ok(match escape {
        'a' => '\x07',
        'b' => '\x08',
        'f' => '\x0c',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\x0b',
        '\\' => '\\',
        '\'' => '\'',
        '"' => '"',
        '?' => '?',
        '0'..='7' | 'x' => {
            let radix = if escape == 'x' { 16 } else { 8 };
            if escape != 'x' {
                *i -= 1;
            }
            let start = *i;
            let mut value = 0u32;
            while let Some(digit) = chars.get(*i).and_then(|c| c.to_digit(radix)) {
                value = value
                    .checked_mul(radix)
                    .and_then(|v| v.checked_add(digit))
                    .ok_or("escape value overflow")?;
                *i += 1;
                if radix == 8 && *i - start == 3 {
                    break;
                }
            }
            if *i == start || value > 127 {
                return Err("numeric escape requires an ASCII value".into());
            }
            char::from_u32(value).ok_or("invalid escape value")?
        }
        _ => return Err("unsupported string escape".into()),
    })
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
