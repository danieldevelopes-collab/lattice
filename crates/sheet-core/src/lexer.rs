//! Turn a formula string into tokens. The parser decides what an identifier
//! *means* (a cell ref, a range, a function, a boolean, a name); the lexer only
//! splits the stream and reads numbers and quoted strings.

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Amp,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    LParen,
    RParen,
    Comma,
    Colon,
    Bang,
}

pub fn lex(input: &str) -> Result<Vec<Tok>, String> {
    let b = input.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();

    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            b'+' => { out.push(Tok::Plus); i += 1; }
            b'-' => { out.push(Tok::Minus); i += 1; }
            b'*' => { out.push(Tok::Star); i += 1; }
            b'/' => { out.push(Tok::Slash); i += 1; }
            b'^' => { out.push(Tok::Caret); i += 1; }
            b'%' => { out.push(Tok::Percent); i += 1; }
            b'&' => { out.push(Tok::Amp); i += 1; }
            b'(' => { out.push(Tok::LParen); i += 1; }
            b')' => { out.push(Tok::RParen); i += 1; }
            b',' => { out.push(Tok::Comma); i += 1; }
            b':' => { out.push(Tok::Colon); i += 1; }
            b'!' => { out.push(Tok::Bang); i += 1; }
            b'=' => { out.push(Tok::Eq); i += 1; }
            b'<' => {
                if b.get(i + 1) == Some(&b'=') { out.push(Tok::Le); i += 2; }
                else if b.get(i + 1) == Some(&b'>') { out.push(Tok::Ne); i += 2; }
                else { out.push(Tok::Lt); i += 1; }
            }
            b'>' => {
                if b.get(i + 1) == Some(&b'=') { out.push(Tok::Ge); i += 2; }
                else { out.push(Tok::Gt); i += 1; }
            }
            b'"' => {
                let (s, ni) = lex_string(input, i)?;
                out.push(Tok::Str(s));
                i = ni;
            }
            _ if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) => {
                let (n, ni) = lex_number(input, i)?;
                out.push(Tok::Num(n));
                i = ni;
            }
            _ if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let start = i;
                while i < b.len() {
                    let d = b[i];
                    if d.is_ascii_alphanumeric() || d == b'_' || d == b'$' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(Tok::Ident(input[start..i].to_string()));
            }
            _ => return Err(format!("unexpected character {:?}", c as char)),
        }
    }
    Ok(out)
}

fn lex_string(input: &str, start: usize) -> Result<(String, usize), String> {
    let b = input.as_bytes();
    let mut i = start + 1; // skip opening quote
    let mut s = String::new();
    while i < b.len() {
        if b[i] == b'"' {
            if b.get(i + 1) == Some(&b'"') {
                s.push('"'); // "" -> literal quote
                i += 2;
            } else {
                return Ok((s, i + 1));
            }
        } else {
            // copy one UTF-8 char
            let ch = input[i..].chars().next().unwrap();
            s.push(ch);
            i += ch.len_utf8();
        }
    }
    Err("unterminated string".into())
}

fn lex_number(input: &str, start: usize) -> Result<(f64, usize), String> {
    let b = input.as_bytes();
    let mut i = start;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            i = j;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    let n: f64 = input[start..i].parse().map_err(|_| format!("bad number {:?}", &input[start..i]))?;
    Ok((n, i))
}
