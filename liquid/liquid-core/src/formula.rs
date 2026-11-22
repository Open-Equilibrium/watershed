use std::cmp::Ordering;
use std::fmt;

pub const MAX_FORMULA: usize = 8_192;
pub const MAX_NESTING: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Div0,
    Value,
    Ref,
    Name,
    Num,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(Failure),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Value(Value),
    Range(Vec<Value>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference(Vec<String>);

impl Reference {
    pub fn ids(&self) -> &[String] {
        &self.0
    }
}

pub trait Resolve {
    fn resolve(&self, reference: &Reference) -> Operand;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syntax {
    pub at: usize,
}

#[derive(Clone, Debug)]
pub struct Formula {
    expr: Expr,
    references: Vec<Reference>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

#[derive(Clone, Debug)]
enum Expr {
    Value(Value),
    Ref(usize),
    Name,
    Call(String, Vec<Expr>),
    Sign { negate: bool, operand: Box<Expr> },
    Percent { times: i32, operand: Box<Expr> },
    Chain(Box<Expr>, Vec<(Op, Expr)>),
}

impl Formula {
    pub fn parse(source: &str) -> Result<Self, Syntax> {
        if source.chars().count() > MAX_FORMULA {
            return Err(Syntax { at: MAX_FORMULA });
        }
        let mut parser = Parser {
            tokens: lex(source)?,
            next: 0,
            depth: 0,
            references: Vec::new(),
        };
        let expr = parser.compare()?;
        match parser.tokens.get(parser.next) {
            Some((_, at)) => Err(Syntax { at: *at }),
            None => Ok(Self {
                expr,
                references: parser.references,
            }),
        }
    }

    pub fn references(&self) -> impl Iterator<Item = &Reference> {
        self.references.iter()
    }

    pub fn evaluate(&self, resolver: &dyn Resolve) -> Value {
        Eval {
            resolver,
            references: &self.references,
        }
        .value(&self.expr)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Name(String),
    Ref(Vec<String>),
    Op(char),
    Le,
    Ge,
    Ne,
}

fn lex(source: &str) -> Result<Vec<(Token, usize)>, Syntax> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let at = i;
        let token = match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
                continue;
            }
            '0'..='9' | '.' => {
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == '.')
                {
                    i += 1;
                }
                if chars.get(i).is_some_and(|c| *c == 'e' || *c == 'E') {
                    let sign =
                        usize::from(chars.get(i + 1).is_some_and(|c| *c == '+' || *c == '-'));
                    if chars.get(i + 1 + sign).is_some_and(char::is_ascii_digit) {
                        i += 1 + sign;
                        while chars.get(i).is_some_and(char::is_ascii_digit) {
                            i += 1;
                        }
                    }
                }
                let text: String = chars[at..i].iter().collect();
                Token::Number(text.parse().map_err(|_| Syntax { at })?)
            }
            '"' => {
                let mut text = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(Syntax { at }),
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            text.push('"');
                            i += 2;
                        }
                        Some('"') => break,
                        Some(c) => {
                            text.push(*c);
                            i += 1;
                        }
                    }
                }
                i += 1;
                Token::Text(text)
            }
            '{' => {
                let end = chars[i..]
                    .iter()
                    .position(|c| *c == '}')
                    .ok_or(Syntax { at })?;
                let inner: String = chars[i + 1..i + end].iter().collect();
                let ids: Vec<String> = inner.split('.').map(str::to_string).collect();
                let valid = ids.iter().all(|id| {
                    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                });
                if !valid {
                    return Err(Syntax { at });
                }
                i += end + 1;
                Token::Ref(ids)
            }
            c if c.is_alphabetic() || c == '_' => {
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                {
                    i += 1;
                }
                Token::Name(chars[at..i].iter().collect::<String>().to_uppercase())
            }
            '<' if chars.get(i + 1) == Some(&'=') => {
                i += 2;
                Token::Le
            }
            '<' if chars.get(i + 1) == Some(&'>') => {
                i += 2;
                Token::Ne
            }
            '>' if chars.get(i + 1) == Some(&'=') => {
                i += 2;
                Token::Ge
            }
            '−' => {
                i += 1;
                Token::Op('-')
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '%' | '=' | '<' | '>' | '(' | ')' | ',' => {
                i += 1;
                Token::Op(c)
            }
            _ => return Err(Syntax { at }),
        };
        tokens.push((token, at));
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<(Token, usize)>,
    next: usize,
    depth: usize,
    references: Vec<Reference>,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.next).map(|(token, _)| token)
    }

    fn here(&self) -> Syntax {
        Syntax {
            at: self.tokens.get(self.next).map_or_else(
                || self.tokens.last().map_or(0, |(_, at)| at + 1),
                |(_, at)| *at,
            ),
        }
    }

    fn take(&mut self, expected: char) -> bool {
        let found = self.peek() == Some(&Token::Op(expected));
        self.next += usize::from(found);
        found
    }

    fn chain(
        &mut self,
        operand: fn(&mut Self) -> Result<Expr, Syntax>,
        op: fn(&Token) -> Option<Op>,
    ) -> Result<Expr, Syntax> {
        let first = operand(self)?;
        let mut rest = Vec::new();
        while let Some(found) = self.peek().and_then(op) {
            self.next += 1;
            rest.push((found, operand(self)?));
        }
        Ok(if rest.is_empty() {
            first
        } else {
            Expr::Chain(Box::new(first), rest)
        })
    }

    fn compare(&mut self) -> Result<Expr, Syntax> {
        self.chain(Self::concat, |token| match token {
            Token::Op('=') => Some(Op::Eq),
            Token::Ne => Some(Op::Ne),
            Token::Op('<') => Some(Op::Lt),
            Token::Le => Some(Op::Le),
            Token::Op('>') => Some(Op::Gt),
            Token::Ge => Some(Op::Ge),
            _ => None,
        })
    }

    fn concat(&mut self) -> Result<Expr, Syntax> {
        self.chain(Self::add, |token| {
            (token == &Token::Op('&')).then_some(Op::Concat)
        })
    }

    fn add(&mut self) -> Result<Expr, Syntax> {
        self.chain(Self::mul, |token| match token {
            Token::Op('+') => Some(Op::Add),
            Token::Op('-') => Some(Op::Sub),
            _ => None,
        })
    }

    fn mul(&mut self) -> Result<Expr, Syntax> {
        self.chain(Self::pow, |token| match token {
            Token::Op('*') => Some(Op::Mul),
            Token::Op('/') => Some(Op::Div),
            _ => None,
        })
    }

    fn pow(&mut self) -> Result<Expr, Syntax> {
        self.chain(Self::percent, |token| {
            (token == &Token::Op('^')).then_some(Op::Pow)
        })
    }

    fn percent(&mut self) -> Result<Expr, Syntax> {
        let operand = self.sign()?;
        let mut times = 0;
        while self.take('%') {
            times += 1;
        }
        Ok(if times == 0 {
            operand
        } else {
            Expr::Percent {
                times,
                operand: Box::new(operand),
            }
        })
    }

    fn sign(&mut self) -> Result<Expr, Syntax> {
        let mut signs = 0;
        let mut negate = false;
        loop {
            if self.take('-') {
                negate = !negate;
            } else if !self.take('+') {
                break;
            }
            signs += 1;
        }
        let operand = self.primary()?;
        Ok(if signs == 0 {
            operand
        } else {
            Expr::Sign {
                negate,
                operand: Box::new(operand),
            }
        })
    }

    fn nested<T>(
        &mut self,
        inner: impl FnOnce(&mut Self) -> Result<T, Syntax>,
    ) -> Result<T, Syntax> {
        if self.depth == MAX_NESTING {
            return Err(self.here());
        }
        self.depth += 1;
        let result = inner(self);
        self.depth -= 1;
        let result = result?;
        if self.take(')') {
            Ok(result)
        } else {
            Err(self.here())
        }
    }

    fn primary(&mut self) -> Result<Expr, Syntax> {
        let error = self.here();
        let Some((token, _)) = self.tokens.get(self.next).cloned() else {
            return Err(error);
        };
        self.next += 1;
        match token {
            Token::Number(number) => Ok(Expr::Value(Value::Number(number))),
            Token::Text(text) => Ok(Expr::Value(Value::Text(text))),
            Token::Ref(ids) => {
                self.references.push(Reference(ids));
                Ok(Expr::Ref(self.references.len() - 1))
            }
            Token::Op('(') => self.nested(Self::compare),
            Token::Name(name) if self.take('(') => {
                let args = self.nested(|parser| {
                    let mut args = Vec::new();
                    if parser.peek() != Some(&Token::Op(')')) {
                        loop {
                            args.push(parser.compare()?);
                            if !parser.take(',') {
                                break;
                            }
                        }
                    }
                    Ok(args)
                })?;
                Ok(Expr::Call(name, args))
            }
            Token::Name(name) => Ok(match name.as_str() {
                "TRUE" => Expr::Value(Value::Bool(true)),
                "FALSE" => Expr::Value(Value::Bool(false)),
                _ => Expr::Name,
            }),
            _ => Err(error),
        }
    }
}

struct Eval<'a> {
    resolver: &'a dyn Resolve,
    references: &'a [Reference],
}

type Outcome<T> = Result<T, Failure>;

impl Eval<'_> {
    fn operand(&self, expr: &Expr) -> Operand {
        match expr {
            Expr::Ref(index) => self.resolver.resolve(&self.references[*index]),
            expr => Operand::Value(self.value(expr)),
        }
    }

    fn value(&self, expr: &Expr) -> Value {
        match expr {
            Expr::Value(value) => value.clone(),
            Expr::Ref(_) => match self.operand(expr) {
                Operand::Value(value) => value,
                Operand::Range(_) => Value::Error(Failure::Value),
            },
            Expr::Name => Value::Error(Failure::Name),
            Expr::Sign { negate, operand } => {
                lift(number(&self.value(operand)).map(|n| if *negate { -n } else { n }))
            }
            Expr::Percent { times, operand } => {
                lift(number(&self.value(operand)).map(|n| n * 0.01f64.powi(*times)))
            }
            Expr::Chain(first, rest) => {
                let mut left = self.value(first);
                for (op, right) in rest {
                    if matches!(left, Value::Error(_)) {
                        break;
                    }
                    left = apply(*op, &left, &self.value(right));
                }
                left
            }
            Expr::Call(name, args) => self.call(name, args).unwrap_or_else(Value::Error),
        }
    }

    fn scalar(&self, expr: &Expr) -> Outcome<Value> {
        match self.value(expr) {
            Value::Error(failure) => Err(failure),
            value => Ok(value),
        }
    }

    fn values(&self, args: &[Expr]) -> Vec<(Value, bool)> {
        args.iter()
            .flat_map(|arg| match self.operand(arg) {
                Operand::Value(value) => vec![(value, true)],
                Operand::Range(values) => values.into_iter().map(|v| (v, false)).collect(),
            })
            .collect()
    }

    fn numbers(&self, args: &[Expr]) -> Outcome<Vec<f64>> {
        let mut numbers = Vec::new();
        for (value, direct) in self.values(args) {
            match value {
                Value::Error(failure) => return Err(failure),
                Value::Number(n) => numbers.push(n),
                value if direct => numbers.push(number(&value)?),
                _ => {}
            }
        }
        Ok(numbers)
    }

    fn range(&self, expr: &Expr) -> Vec<Value> {
        match self.operand(expr) {
            Operand::Value(value) => vec![value],
            Operand::Range(values) => values,
        }
    }

    fn matching(&self, args: &[Expr]) -> Outcome<Vec<f64>> {
        let tested = self.range(&args[0]);
        let criterion = Criterion::new(self.scalar(&args[1])?);
        let summed = args
            .get(2)
            .map_or_else(|| tested.clone(), |arg| self.range(arg));
        let mut numbers = Vec::new();
        for (index, value) in tested.iter().enumerate() {
            if criterion.matches(value) {
                match summed.get(index) {
                    Some(Value::Number(n)) => numbers.push(*n),
                    Some(Value::Error(failure)) => return Err(*failure),
                    _ => {}
                }
            }
        }
        Ok(numbers)
    }

    fn number(&self, expr: &Expr) -> Outcome<f64> {
        number(&self.scalar(expr)?)
    }

    fn text(&self, expr: &Expr) -> Outcome<String> {
        text(&self.scalar(expr)?)
    }

    fn truth(&self, expr: &Expr) -> Outcome<bool> {
        match self.scalar(expr)? {
            Value::Text(_) => Err(Failure::Value),
            value => Ok(number(&value)? != 0.0),
        }
    }

    fn call(&self, name: &str, args: &[Expr]) -> Outcome<Value> {
        let (min, max) = arity(name).ok_or(Failure::Name)?;
        if args.len() < min || args.len() > max {
            return Err(Failure::Value);
        }
        let arg = |index: usize| &args[index];
        let count = |index: usize, default: f64| {
            args.get(index)
                .map_or(Ok(default), |arg| self.number(arg))
                .map(f64::trunc)
        };
        let n = Value::Number;
        Ok(match name {
            "SUM" => n(self.numbers(args)?.iter().sum()),
            "PRODUCT" => n(self.numbers(args)?.iter().product()),
            "AVERAGE" => average(&self.numbers(args)?)?,
            "MIN" => n(self
                .numbers(args)?
                .into_iter()
                .reduce(f64::min)
                .unwrap_or(0.0)),
            "MAX" => n(self
                .numbers(args)?
                .into_iter()
                .reduce(f64::max)
                .unwrap_or(0.0)),
            "MEDIAN" => {
                let mut numbers = self.numbers(args)?;
                numbers.sort_by(f64::total_cmp);
                let middle = numbers.len() / 2;
                match numbers.len() {
                    0 => return Err(Failure::Num),
                    len if len % 2 == 0 => n((numbers[middle - 1] + numbers[middle]) / 2.0),
                    _ => n(numbers[middle]),
                }
            }
            "COUNT" => n(self
                .values(args)
                .iter()
                .filter(|(value, direct)| match value {
                    Value::Number(_) => true,
                    Value::Error(_) | Value::Blank => false,
                    value => *direct && number(value).is_ok(),
                })
                .count() as f64),
            "COUNTA" => n(self
                .values(args)
                .iter()
                .filter(|(value, _)| *value != Value::Blank)
                .count() as f64),
            "COUNTBLANK" => n(self
                .range(arg(0))
                .iter()
                .filter(|value| {
                    matches!(value, Value::Blank) || *value == &Value::Text(String::new())
                })
                .count() as f64),
            "SUMIF" => n(self.matching(args)?.iter().sum()),
            "COUNTIF" => {
                let criterion = Criterion::new(self.scalar(arg(1))?);
                n(self
                    .range(arg(0))
                    .iter()
                    .filter(|v| criterion.matches(v))
                    .count() as f64)
            }
            "AVERAGEIF" => average(&self.matching(args)?)?,
            "IF" => {
                if self.truth(arg(0))? {
                    self.value(arg(1))
                } else {
                    args.get(2)
                        .map_or(Value::Bool(false), |arg| self.value(arg))
                }
            }
            "IFERROR" => match self.value(arg(0)) {
                Value::Error(_) => self.value(arg(1)),
                value => value,
            },
            "AND" | "OR" => {
                let mut found = Vec::new();
                for (value, direct) in self.values(args) {
                    match value {
                        Value::Error(failure) => return Err(failure),
                        Value::Text(_) if direct => return Err(Failure::Value),
                        Value::Text(_) | Value::Blank => {}
                        value => found.push(number(&value)? != 0.0),
                    }
                }
                if found.is_empty() {
                    return Err(Failure::Value);
                }
                Value::Bool(if name == "AND" {
                    found.iter().all(|b| *b)
                } else {
                    found.iter().any(|b| *b)
                })
            }
            "NOT" => Value::Bool(!self.truth(arg(0))?),
            "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
                let value = self.number(arg(0))?;
                let scale = 10f64.powf(self.number(arg(1))?.trunc());
                let scaled = round15(value * scale);
                let rounded = match name {
                    "ROUND" => scaled.round(),
                    "ROUNDUP" => scaled.abs().ceil().copysign(scaled),
                    _ => scaled.trunc(),
                };
                finite(round15(rounded / scale))?
            }
            "ABS" => n(self.number(arg(0))?.abs()),
            "INT" => n(self.number(arg(0))?.floor()),
            "MOD" => {
                let (a, b) = (self.number(arg(0))?, self.number(arg(1))?);
                if b == 0.0 {
                    return Err(Failure::Div0);
                }
                finite(a - b * (a / b).floor())?
            }
            "POWER" => power(self.number(arg(0))?, self.number(arg(1))?)?,
            "SQRT" => finite(self.number(arg(0))?.sqrt())?,
            "LEN" => n(self.text(arg(0))?.chars().count() as f64),
            "UPPER" => Value::Text(self.text(arg(0))?.to_uppercase()),
            "LOWER" => Value::Text(self.text(arg(0))?.to_lowercase()),
            "TRIM" => Value::Text(
                self.text(arg(0))?
                    .split(' ')
                    .filter(|w| !w.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            "LEFT" | "RIGHT" => {
                let chars: Vec<char> = self.text(arg(0))?.chars().collect();
                let take = chars.len().min(whole(count(1, 1.0)?)?);
                Value::Text(if name == "LEFT" {
                    chars[..take].iter().collect()
                } else {
                    chars[chars.len() - take..].iter().collect()
                })
            }
            "MID" => {
                let chars: Vec<char> = self.text(arg(0))?.chars().collect();
                let start = whole(count(1, 1.0)? - 1.0)?;
                let take = whole(count(2, 0.0)?)?;
                Value::Text(chars.iter().skip(start).take(take).collect())
            }
            "CONCAT" => {
                let mut joined = String::new();
                for (value, _) in self.values(args) {
                    joined.push_str(&text(&value)?);
                }
                Value::Text(joined)
            }
            "INDEX" => {
                let row = whole(count(1, 1.0)? - 1.0)?;
                self.range(arg(0))
                    .into_iter()
                    .nth(row)
                    .ok_or(Failure::Ref)?
            }
            "ISBLANK" => Value::Bool(self.value(arg(0)) == Value::Blank),
            "ISNUMBER" => Value::Bool(matches!(self.value(arg(0)), Value::Number(_))),
            "ISTEXT" => Value::Bool(matches!(self.value(arg(0)), Value::Text(_))),
            "ISERROR" => Value::Bool(matches!(self.value(arg(0)), Value::Error(_))),
            _ => return Err(Failure::Name),
        })
    }
}

fn arity(name: &str) -> Option<(usize, usize)> {
    Some(match name {
        "SUM" | "PRODUCT" | "AVERAGE" | "MIN" | "MAX" | "MEDIAN" | "COUNT" | "COUNTA" | "AND"
        | "OR" | "CONCAT" => (1, 255),
        "COUNTBLANK" | "NOT" | "ABS" | "INT" | "SQRT" | "LEN" | "UPPER" | "LOWER" | "TRIM"
        | "ISBLANK" | "ISNUMBER" | "ISTEXT" | "ISERROR" => (1, 1),
        "SUMIF" | "AVERAGEIF" | "IF" => (2, 3),
        "COUNTIF" | "IFERROR" | "ROUND" | "ROUNDUP" | "ROUNDDOWN" | "MOD" | "POWER" => (2, 2),
        "LEFT" | "RIGHT" | "INDEX" => (1, 2),
        "MID" => (3, 3),
        _ => return None,
    })
}

fn lift(outcome: Outcome<f64>) -> Value {
    outcome.and_then(finite).unwrap_or_else(Value::Error)
}

fn finite(n: f64) -> Outcome<Value> {
    if n.is_finite() {
        Ok(Value::Number(n + 0.0))
    } else {
        Err(Failure::Num)
    }
}

fn whole(n: f64) -> Outcome<usize> {
    if (0.0..=f64::from(u32::MAX)).contains(&n) {
        Ok(n as usize)
    } else {
        Err(Failure::Value)
    }
}

fn average(numbers: &[f64]) -> Outcome<Value> {
    if numbers.is_empty() {
        return Err(Failure::Div0);
    }
    finite(numbers.iter().sum::<f64>() / numbers.len() as f64)
}

fn power(base: f64, exponent: f64) -> Outcome<Value> {
    if base == 0.0 && exponent == 0.0 {
        return Err(Failure::Num);
    }
    if base == 0.0 && exponent < 0.0 {
        return Err(Failure::Div0);
    }
    finite(base.powf(exponent))
}

fn number(value: &Value) -> Outcome<f64> {
    match value {
        Value::Blank => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::Text(text) => numeric(text).ok_or(Failure::Value),
        Value::Error(failure) => Err(*failure),
    }
}

fn numeric(text: &str) -> Option<f64> {
    let text = text.trim();
    let plain = !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    plain.then(|| text.parse().ok()).flatten()
}

fn text(value: &Value) -> Outcome<String> {
    match value {
        Value::Error(failure) => Err(*failure),
        Value::Blank => Ok(String::new()),
        value => Ok(value.to_string()),
    }
}

fn apply(op: Op, left: &Value, right: &Value) -> Value {
    let arithmetic = |f: fn(f64, f64) -> Outcome<Value>| {
        number(left)
            .and_then(|a| number(right).and_then(|b| f(a, b)))
            .unwrap_or_else(Value::Error)
    };
    let compared = |test: fn(Ordering) -> bool| match (left, right) {
        (Value::Error(failure), _) | (_, Value::Error(failure)) => Value::Error(*failure),
        _ => Value::Bool(test(compare(left, right))),
    };
    match op {
        Op::Add => arithmetic(|a, b| finite(a + b)),
        Op::Sub => arithmetic(|a, b| finite(a - b)),
        Op::Mul => arithmetic(|a, b| finite(a * b)),
        Op::Div => arithmetic(|a, b| {
            if b == 0.0 {
                Err(Failure::Div0)
            } else {
                finite(a / b)
            }
        }),
        Op::Pow => arithmetic(power),
        Op::Concat => match (text(left), text(right)) {
            (Ok(a), Ok(b)) => Value::Text(a + &b),
            (Err(failure), _) | (_, Err(failure)) => Value::Error(failure),
        },
        Op::Eq => compared(Ordering::is_eq),
        Op::Ne => compared(Ordering::is_ne),
        Op::Lt => compared(Ordering::is_lt),
        Op::Le => compared(Ordering::is_le),
        Op::Gt => compared(Ordering::is_gt),
        Op::Ge => compared(Ordering::is_ge),
    }
}

fn compare(left: &Value, right: &Value) -> Ordering {
    let rank = |value: &Value| match value {
        Value::Number(_) | Value::Blank => 0,
        Value::Text(_) => 1,
        _ => 2,
    };
    match (left, right) {
        (Value::Text(text), Value::Blank) | (Value::Blank, Value::Text(text))
            if text.is_empty() =>
        {
            Ordering::Equal
        }
        (Value::Text(a), Value::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(_) | Value::Blank, Value::Number(_) | Value::Blank) => number(left)
            .unwrap_or(0.0)
            .total_cmp(&number(right).unwrap_or(0.0)),
        _ => rank(left).cmp(&rank(right)),
    }
}

struct Criterion {
    op: Op,
    against: Value,
}

impl Criterion {
    fn new(value: Value) -> Self {
        let Value::Text(text) = &value else {
            return Self {
                op: Op::Eq,
                against: value,
            };
        };
        let (op, rest) = [
            ("<=", Op::Le),
            (">=", Op::Ge),
            ("<>", Op::Ne),
            ("<", Op::Lt),
            (">", Op::Gt),
            ("=", Op::Eq),
        ]
        .into_iter()
        .find_map(|(prefix, op)| text.strip_prefix(prefix).map(|rest| (op, rest)))
        .unwrap_or((Op::Eq, text));
        let against = match numeric(rest) {
            Some(n) => Value::Number(n),
            None if rest.is_empty() => Value::Blank,
            None => Value::Text(rest.to_string()),
        };
        Self { op, against }
    }

    fn matches(&self, value: &Value) -> bool {
        let found = match (&self.against, value) {
            (Value::Text(pattern), Value::Text(text)) if matches!(self.op, Op::Eq | Op::Ne) => {
                let hit = wildcard(&pattern.to_lowercase(), &text.to_lowercase());
                return hit == (self.op == Op::Eq);
            }
            (Value::Number(_), Value::Number(_))
            | (Value::Text(_), Value::Text(_))
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Blank, Value::Blank) => apply(self.op, value, &self.against),
            _ => return self.op == Op::Ne,
        };
        found == Value::Bool(true)
    }
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut reachable = vec![false; text.len() + 1];
    reachable[0] = true;
    for p in &pattern {
        let mut next = vec![false; text.len() + 1];
        for i in 0..=text.len() {
            if !reachable[i] {
                continue;
            }
            match p {
                '*' => next[i..].iter_mut().for_each(|r| *r = true),
                '?' if i < text.len() => next[i + 1] = true,
                c if i < text.len() && text[i] == *c => next[i + 1] = true,
                _ => {}
            }
        }
        reachable = next;
    }
    reachable[text.len()]
}

fn round15(n: f64) -> f64 {
    format!("{n:.14e}").parse().unwrap_or(n)
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Div0 => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::Num => "#NUM!",
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => Ok(()),
            Self::Number(n) => {
                let shown = round15(*n) + 0.0;
                if shown == 0.0 || (1e-9..1e15).contains(&shown.abs()) {
                    return write!(f, "{shown}");
                }
                let scientific = format!("{shown:E}");
                match scientific.split_once('E') {
                    Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                        write!(f, "{mantissa}E+{exponent}")
                    }
                    _ => f.write_str(&scientific),
                }
            }
            Self::Text(text) => f.write_str(text),
            Self::Bool(b) => f.write_str(if *b { "TRUE" } else { "FALSE" }),
            Self::Error(failure) => failure.fmt(f),
        }
    }
}
