//! Turn tokens into an `Expr` tree by recursive descent.
//!
//! Precedence, highest to lowest, follows Excel — including its quirk that
//! unary minus binds *tighter* than `^`, so `-2^2` parses as `(-2)^2 = 4`:
//!   primary · postfix `%` · unary `-`/`+` · `^` · `* /` · `+ -` · `&` · comparisons.
//! `^` is left-associative, matching Excel (`2^3^2 = (2^3)^2`).

use crate::ast::{BinOp, Expr, RangeRef, RefPart, Reference, UnaryOp};
use crate::cellref::parse_a1;
use crate::lexer::{lex, Tok};

/// Parse a formula (with or without a leading `=`) into an `Expr`.
pub fn parse_formula(input: &str) -> Result<Expr, String> {
    let body = input.strip_prefix('=').unwrap_or(input);
    let toks = lex(body)?;
    let mut p = Parser { toks, pos: 0 };
    let e = p.comparison()?;
    if p.pos != p.toks.len() {
        return Err("unexpected trailing input".into());
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == Some(t) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: Tok) -> Result<(), String> {
        if self.eat(&t) {
            Ok(())
        } else {
            Err(format!("expected {t:?}, found {:?}", self.peek()))
        }
    }

    fn comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.concat()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Eq) => BinOp::Eq,
                Some(Tok::Ne) => BinOp::Ne,
                Some(Tok::Lt) => BinOp::Lt,
                Some(Tok::Gt) => BinOp::Gt,
                Some(Tok::Le) => BinOp::Le,
                Some(Tok::Ge) => BinOp::Ge,
                _ => break,
            };
            self.pos += 1;
            let right = self.concat()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn concat(&mut self) -> Result<Expr, String> {
        let mut left = self.addsub()?;
        while self.eat(&Tok::Amp) {
            let right = self.addsub()?;
            left = Expr::Binary(BinOp::Concat, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn addsub(&mut self) -> Result<Expr, String> {
        let mut left = self.muldiv()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Plus) => BinOp::Add,
                Some(Tok::Minus) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.muldiv()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn muldiv(&mut self) -> Result<Expr, String> {
        let mut left = self.pow()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Star) => BinOp::Mul,
                Some(Tok::Slash) => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let right = self.pow()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn pow(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while self.eat(&Tok::Caret) {
            let right = self.unary()?;
            left = Expr::Binary(BinOp::Pow, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.pos += 1;
                Ok(Expr::Unary(UnaryOp::Neg, Box::new(self.unary()?)))
            }
            Some(Tok::Plus) => {
                self.pos += 1;
                Ok(Expr::Unary(UnaryOp::Pos, Box::new(self.unary()?)))
            }
            _ => self.postfix(),
        }
    }
    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        while self.eat(&Tok::Percent) {
            e = Expr::Unary(UnaryOp::Percent, Box::new(e));
        }
        Ok(e)
    }
    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Number(n)),
            Some(Tok::Str(s)) => Ok(Expr::Text(s)),
            Some(Tok::LParen) => {
                let e = self.comparison()?;
                self.expect(Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(id)) => self.ident(id),
            other => Err(format!("unexpected token {other:?}")),
        }
    }
    fn ident(&mut self, id: String) -> Result<Expr, String> {
        // function call: IDENT ( args )
        if self.peek() == Some(&Tok::LParen) {
            self.pos += 1;
            let mut args = Vec::new();
            if self.peek() != Some(&Tok::RParen) {
                loop {
                    args.push(self.comparison()?);
                    if self.eat(&Tok::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.expect(Tok::RParen)?;
            return Ok(Expr::Func(id.to_ascii_uppercase(), args));
        }
        // sheet-qualified: Sheet ! reference
        if self.eat(&Tok::Bang) {
            let next_id = match self.next() {
                Some(Tok::Ident(s)) => s,
                other => return Err(format!("expected a reference after '!', found {other:?}")),
            };
            return self.ref_or_range(Some(id), &next_id);
        }
        // boolean literal
        match id.to_ascii_uppercase().as_str() {
            "TRUE" => return Ok(Expr::Bool(true)),
            "FALSE" => return Ok(Expr::Bool(false)),
            _ => {}
        }
        self.ref_or_range(None, &id)
    }
    fn ref_or_range(&mut self, sheet: Option<String>, first: &str) -> Result<Expr, String> {
        let a = match parse_a1(first) {
            Some(p) => RefPart { cell: p.cell, abs_col: p.abs_col, abs_row: p.abs_row },
            None => {
                if sheet.is_some() {
                    return Err(format!("invalid reference {first:?}"));
                }
                return Ok(Expr::Name(first.to_string()));
            }
        };
        if self.eat(&Tok::Colon) {
            let b_id = match self.next() {
                Some(Tok::Ident(s)) => s,
                other => return Err(format!("expected a reference after ':', found {other:?}")),
            };
            let bp = parse_a1(&b_id).ok_or_else(|| format!("invalid reference {b_id:?}"))?;
            let b = RefPart { cell: bp.cell, abs_col: bp.abs_col, abs_row: bp.abs_row };
            Ok(Expr::Range(RangeRef { sheet, a, b }))
        } else {
            Ok(Expr::Ref(Reference { sheet, part: a }))
        }
    }
}
