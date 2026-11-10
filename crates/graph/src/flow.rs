//! The condition language of `if_switch`.
//!
//! A tiny grammar over variables, the frame size, and literals. No eval, no
//! function calls, no arithmetic. The worker evaluates; this crate only
//! validates, so a bad condition is named before a job is ever spawned.
//!
//! ```text
//! or_expr  := and_expr ('or' and_expr)*
//! and_expr := not_expr ('and' not_expr)*
//! not_expr := 'not' not_expr | cmp
//! cmp      := operand (('=='|'!='|'<'|'<='|'>'|'>=') operand)?
//! operand  := '${name}' | 'rows' | 'columns' | number | string | true | false
//!            | '(' or_expr ')'
//! ```

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Number,
    Str,
    Var,
    /// rows, columns, true, false
    Operand,
    Word(&'static str),
    Cmp(&'static str),
    LParen,
    RParen,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Tok,
    pos: usize,
    text: String,
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_rest(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    let mut tokens = Vec::new();
    while index < bytes.len() {
        let start = index;
        match bytes[index] {
            b' ' | b'\t' | b'\n' | b'\r' => index += 1,
            b'(' => {
                tokens.push(Token {
                    kind: Tok::LParen,
                    pos: start,
                    text: "(".into(),
                });
                index += 1;
            }
            b')' => {
                tokens.push(Token {
                    kind: Tok::RParen,
                    pos: start,
                    text: ")".into(),
                });
                index += 1;
            }
            b'\'' | b'"' => {
                let quote = bytes[index] as char;
                let close = text[start + 1..]
                    .chars()
                    .position(|ch| ch == quote)
                    .map(|offset| start + 1 + offset);
                match close {
                    Some(close) => {
                        tokens.push(Token {
                            kind: Tok::Str,
                            pos: start,
                            text: text[start..close + 1].to_string(),
                        });
                        index = close + 1;
                    }
                    None => return Err(format!("unterminated string at {start}")),
                }
            }
            b'$' => {
                if !text[index..].starts_with("${") {
                    return Err(format!("unexpected '$' at {start}"));
                }
                let Some(close) = text[start + 2..].find('}') else {
                    return Err(format!("unterminated variable at {start}"));
                };
                let name = &text[start + 2..start + 2 + close];
                if name.is_empty()
                    || !name.chars().all(is_ident_rest)
                    || !is_ident_start(name.chars().next().expect("not empty"))
                {
                    return Err(format!("not a variable name at {}", start + 2));
                }
                tokens.push(Token {
                    kind: Tok::Var,
                    pos: start,
                    text: name.to_string(),
                });
                index = start + 2 + close + 1;
            }
            b'=' => {
                if text[index..].starts_with("==") {
                    tokens.push(Token {
                        kind: Tok::Cmp("=="),
                        pos: start,
                        text: "==".into(),
                    });
                    index += 2;
                } else {
                    return Err(format!("unexpected '=' at {start}"));
                }
            }
            b'!' => {
                if text[index..].starts_with("!=") {
                    tokens.push(Token {
                        kind: Tok::Cmp("!="),
                        pos: start,
                        text: "!=".into(),
                    });
                    index += 2;
                } else {
                    return Err(format!("unexpected '!' at {start}"));
                }
            }
            b'<' | b'>' => {
                let two = text[index..].starts_with("<=") || text[index..].starts_with(">=");
                let op = if two {
                    &text[index..index + 2]
                } else {
                    &text[index..index + 1]
                };
                let kind = match op {
                    "<=" => Tok::Cmp("<="),
                    ">=" => Tok::Cmp(">="),
                    "<" => Tok::Cmp("<"),
                    _ => Tok::Cmp(">"),
                };
                tokens.push(Token {
                    kind,
                    pos: start,
                    text: op.to_string(),
                });
                index += op.len();
            }
            ch if ch.is_ascii_digit() => {
                while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.')
                {
                    index += 1;
                }
                tokens.push(Token {
                    kind: Tok::Number,
                    pos: start,
                    text: text[start..index].to_string(),
                });
            }
            ch if is_ident_start(ch as char) => {
                while index < bytes.len() && is_ident_rest(bytes[index] as char) {
                    index += 1;
                }
                let word = &text[start..index];
                let kind = match word {
                    "and" => Tok::Word("and"),
                    "or" => Tok::Word("or"),
                    "not" => Tok::Word("not"),
                    "rows" | "columns" | "true" | "false" => Tok::Operand,
                    other => return Err(format!("unknown name \"{other}\" at {start}")),
                };
                tokens.push(Token {
                    kind,
                    pos: start,
                    text: word.to_string(),
                });
            }
            other => {
                let ch = text[start..].chars().next().unwrap_or('?');
                let _ = other;
                return Err(format!("unexpected character '{ch}' at {start}"));
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    len: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn end_position(&self) -> usize {
        self.len
    }

    fn parse_or(&mut self) -> Result<(), String> {
        self.parse_and()?;
        while let Some(token) = self.peek() {
            if token.kind != Tok::Word("or") {
                break;
            }
            self.index += 1;
            self.parse_and()?;
        }
        Ok(())
    }

    fn parse_and(&mut self) -> Result<(), String> {
        self.parse_not()?;
        while let Some(token) = self.peek() {
            if token.kind != Tok::Word("and") {
                break;
            }
            self.index += 1;
            self.parse_not()?;
        }
        Ok(())
    }

    fn parse_not(&mut self) -> Result<(), String> {
        if let Some(token) = self.peek()
            && token.kind == Tok::Word("not")
        {
            self.index += 1;
            return self.parse_not();
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<(), String> {
        self.parse_operand()?;
        if let Some(token) = self.peek()
            && matches!(token.kind, Tok::Cmp(_))
        {
            self.index += 1;
            self.parse_operand()?;
        }
        Ok(())
    }

    fn parse_operand(&mut self) -> Result<(), String> {
        let Some(token) = self.peek() else {
            return Err(format!("expected a value at {}", self.end_position()));
        };
        match &token.kind {
            Tok::Number | Tok::Str | Tok::Var | Tok::Operand => {
                self.index += 1;
                Ok(())
            }
            Tok::LParen => {
                let open = token.pos;
                self.index += 1;
                self.parse_or()?;
                match self.peek() {
                    Some(token) if token.kind == Tok::RParen => {
                        self.index += 1;
                        Ok(())
                    }
                    _ => Err(format!("unbalanced parenthesis at {open}")),
                }
            }
            Tok::Word(word) => Err(format!("unexpected \"{word}\" at {}", token.pos)),
            Tok::RParen => Err(format!("unexpected \")\" at {}", token.pos)),
            Tok::Cmp(op) => Err(format!("unexpected \"{op}\" at {}", token.pos)),
        }
    }
}

/// Checks the text against the grammar. The error is short and names the
/// character position the parser stopped at.
pub fn validate_condition(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("condition is empty".to_string());
    }
    let tokens = tokenize(text)?;
    let mut parser = Parser {
        tokens,
        index: 0,
        len: text.len(),
    };
    parser.parse_or()?;
    if let Some(token) = parser.peek() {
        return Err(format!("unexpected \"{}\" at {}", token.text, token.pos));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) {
        validate_condition(text).unwrap_or_else(|err| panic!("{text}: {err}"));
    }

    fn err(text: &str) -> String {
        validate_condition(text).unwrap_err()
    }

    #[test]
    fn accepts_the_documented_forms() {
        ok("${mode} == \"full\"");
        ok("${mode} == 'full'");
        ok("rows > 0");
        ok("not rows > 0");
        ok("rows > 0 and columns >= 2");
        ok("rows == 0 or ${flag} and not columns == 1");
        ok("(${mode} == \"full\" or rows > 10) and not ${off}");
        ok("true");
        ok("false or ${on}");
        ok("1.5 <= ${ratio}");
        ok("columns != 0");
    }

    #[test]
    fn rejects_code_injection_and_broken_text() {
        assert!(err("__import__('os')").contains("unknown name \"__import__\" at 0"));
        assert!(err("eval('x')").contains("at 0"));
        assert!(err("rows >").contains("expected a value at"));
        assert!(err("(rows > 0").contains("unbalanced parenthesis at 0"));
        assert!(err("((rows > 0)").contains("unbalanced parenthesis at 0"));
        assert!(err("rows > 0)").contains("unexpected \")\" at 8"));
        assert!(err("rows > 0 and").contains("expected a value at"));
        assert!(err("and rows").contains("unexpected \"and\" at 0"));
        assert!(err("rows = 0").contains("unexpected '=' at 5"));
        assert!(err("'open").contains("unterminated string at 0"));
        assert!(err("${1bad} == 2").contains("not a variable name at 2"));
        assert!(err("").contains("condition is empty"));
        assert!(err("   ").contains("condition is empty"));
        assert!(err("rows ~ 0").contains("unexpected character '~' at 5"));
    }
}
