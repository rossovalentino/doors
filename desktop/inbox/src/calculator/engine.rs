//! Calculator's Standard mode: immediate execution (2 + 3 × 4 = 20), the
//! expression line ("5 ×", "sqr(3) =", "negate(5)"), repeated "=", percent
//! of the pending operand, memory and history, and the error states Windows
//! shows ("Cannot divide by zero", "Invalid input", "Overflow").
use super::number::{Fault, Number, group};
use alloc::{format, string::String, vec::Vec};

/// Digits a typed entry may hold.
pub const MAX_INPUT: usize = 16;
/// History and memory items kept (Windows keeps the latest first).
pub const MAX_ITEMS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
}
impl Op {
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "\u{2212}",
            Self::Mul => "\u{00d7}",
            Self::Div => "\u{00f7}",
        }
    }
    fn apply(self, a: Number, b: Number) -> Result<Number, Fault> {
        match self {
            Self::Add => a.try_add(b),
            Self::Sub => a.try_sub(b),
            Self::Mul => a.try_mul(b),
            Self::Div => a.try_div(b),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Digit(u8),
    Point,
    Op(Op),
    Equals,
    Percent,
    ClearEntry,
    Clear,
    Backspace,
    Negate,
    Reciprocal,
    Square,
    SquareRoot,
    MemoryClear,
    MemoryRecall,
    MemoryAdd,
    MemorySubtract,
    MemoryStore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryItem {
    pub expression: String,
    pub result: Number,
}

/// The first operand, as the expression shows it ("√(9)"), and the
/// operator waiting for the second.
#[derive(Clone, Debug)]
struct Pending {
    value: Number,
    op: Op,
    text: String,
}

#[derive(Clone, Debug)]
pub struct Calculator {
    /// The number being typed, as typed ("0.50", "-12"); `None` once a
    /// computed value is shown.
    typing: Option<String>,
    value: Number,
    /// The operand's function text ("sqr(3)", "negate(5)").
    function: Option<String>,
    /// First operand and pending operator.
    pending: Option<Pending>,
    /// Whether an operand has been entered since the operator.
    operand: bool,
    /// The last "= " operation, repeated by another "=".
    repeat: Option<(Op, Number)>,
    expression: String,
    error: Option<&'static str>,
    evaluated: bool,
    pub history: Vec<HistoryItem>,
    pub memory: Vec<Number>,
}

impl Default for Calculator {
    fn default() -> Self {
        Self::new()
    }
}

fn fault_text(f: Fault) -> &'static str {
    match f {
        Fault::DivideByZero => "Cannot divide by zero",
        Fault::Invalid => "Invalid input",
        Fault::Overflow => "Overflow",
    }
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            typing: None,
            value: Number::ZERO,
            function: None,
            pending: None,
            operand: false,
            repeat: None,
            expression: String::new(),
            error: None,
            evaluated: false,
            history: Vec::new(),
            memory: Vec::new(),
        }
    }
    /// The large display line.
    pub fn display(&self) -> String {
        if let Some(e) = self.error {
            return e.into();
        }
        match &self.typing {
            Some(t) => typed_display(t),
            None => self.value.display(true),
        }
    }
    /// The expression line above it.
    pub fn expression(&self) -> &str {
        &self.expression
    }
    pub fn error(&self) -> bool {
        self.error.is_some()
    }
    pub fn value(&self) -> Number {
        self.value
    }
    /// Operators, functions, the point, negate and memory are disabled
    /// while an error is shown.
    pub fn enabled(&self, c: Command) -> bool {
        match c {
            Command::Digit(_)
            | Command::ClearEntry
            | Command::Clear
            | Command::Backspace
            | Command::Equals => true,
            Command::MemoryClear | Command::MemoryRecall => {
                !self.error() && !self.memory.is_empty()
            }
            _ => !self.error(),
        }
    }
    fn reset(&mut self) {
        let history = core::mem::take(&mut self.history);
        let memory = core::mem::take(&mut self.memory);
        *self = Self {
            history,
            memory,
            ..Self::new()
        };
    }
    fn fail(&mut self, f: Fault, expression: String) {
        self.error = Some(fault_text(f));
        self.expression = expression;
        self.typing = None;
        self.function = None;
        self.pending = None;
        self.repeat = None;
        self.evaluated = true;
    }
    /// The operand's text in an expression.
    fn operand_text(&self) -> String {
        match &self.function {
            Some(f) => f.clone(),
            None => self.value.display(true),
        }
    }
    fn prefix(&self) -> String {
        match &self.pending {
            Some(p) => format!("{} {} ", p.text, p.op.symbol()),
            None => String::new(),
        }
    }
    /// Commits a typed entry to `value`.
    fn commit(&mut self) {
        if let Some(t) = self.typing.take() {
            self.value = Number::parse(&t).unwrap_or(Number::ZERO);
        }
    }
    pub fn press(&mut self, c: Command) {
        if !self.enabled(c) {
            return;
        }
        if self.error.is_some() {
            // Anything allowed during an error starts over first.
            self.reset();
            if matches!(
                c,
                Command::ClearEntry | Command::Clear | Command::Backspace | Command::Equals
            ) {
                return;
            }
        }
        match c {
            Command::Digit(d) => self.digit(d),
            Command::Point => self.point(),
            Command::Op(op) => self.op(op),
            Command::Equals => self.equals(),
            Command::Percent => self.percent(),
            Command::ClearEntry => {
                self.typing = None;
                self.value = Number::ZERO;
                self.function = None;
                if self.evaluated {
                    self.expression.clear();
                    self.evaluated = false;
                }
            }
            Command::Clear => self.reset(),
            Command::Backspace => {
                if let Some(t) = &mut self.typing {
                    t.pop();
                    if t.is_empty() || t == "-" || t == "-0" {
                        *t = "0".into();
                    }
                } else if self.evaluated {
                    self.expression.clear();
                }
            }
            Command::Negate => self.negate(),
            Command::Reciprocal => self.function("1/", Number::recip),
            Command::Square => self.function("sqr", Number::sqr),
            Command::SquareRoot => self.function("\u{221a}", Number::sqrt),
            Command::MemoryClear => self.memory.clear(),
            Command::MemoryRecall => {
                self.typing = None;
                self.function = None;
                self.value = self.memory[0];
                self.operand = true;
            }
            Command::MemoryStore => {
                self.commit();
                if self.memory.len() == MAX_ITEMS {
                    self.memory.pop();
                }
                self.memory.insert(0, self.value);
                self.end_entry();
            }
            Command::MemoryAdd | Command::MemorySubtract => {
                self.commit();
                let v = if c == Command::MemoryAdd {
                    self.value
                } else {
                    self.value.negated()
                };
                match self.memory.first().copied() {
                    Some(m) => match m.try_add(v) {
                        Ok(sum) => self.memory[0] = sum,
                        Err(f) => self.fail(f, self.expression.clone()),
                    },
                    None => self.memory.insert(0, v),
                }
                self.end_entry();
            }
        }
    }
    /// After memory commands the shown value is no longer being typed.
    fn end_entry(&mut self) {
        self.typing = None;
    }
    fn digit(&mut self, d: u8) {
        if self.evaluated && self.pending.is_none() {
            self.expression.clear();
            self.function = None;
        }
        self.evaluated = false;
        let t = match &mut self.typing {
            Some(t) => t,
            None => {
                self.function = None;
                self.operand = true;
                if self.pending.is_none() {
                    self.expression.clear();
                } else {
                    self.expression = self.prefix();
                }
                self.typing.insert(String::new())
            }
        };
        let count = t.bytes().filter(u8::is_ascii_digit).count();
        if t == "0" || t == "-0" {
            t.pop();
        } else if count >= MAX_INPUT {
            return;
        }
        t.push(char::from(b'0' + d));
    }
    fn point(&mut self) {
        if self.typing.is_none() {
            self.digit(0);
        }
        if let Some(t) = &mut self.typing
            && !t.contains('.')
        {
            t.push('.');
        }
    }
    fn op(&mut self, op: Op) {
        self.commit();
        let entered = self.operand || self.function.is_some();
        match self.pending.take() {
            // Immediate execution: the pending operation runs now and its
            // result is the next first operand ("2 + 3 ×" shows "5 ×").
            Some(p) if entered => match p.op.apply(p.value, self.value) {
                Ok(r) => {
                    self.value = r;
                    self.pending = Some(Pending {
                        value: r,
                        op,
                        text: r.display(true),
                    });
                }
                Err(f) => {
                    let e = format!(
                        "{} {} {} {}",
                        p.text,
                        p.op.symbol(),
                        self.operand_text(),
                        op.symbol()
                    );
                    return self.fail(f, e);
                }
            },
            // Another operator straight away replaces the pending one.
            Some(p) => self.pending = Some(Pending { op, ..p }),
            // The first operand keeps its function text ("sqr(3) +").
            None => {
                self.pending = Some(Pending {
                    value: self.value,
                    op,
                    text: self.operand_text(),
                })
            }
        }
        self.expression = self.prefix();
        self.function = None;
        self.operand = false;
        self.evaluated = false;
    }
    fn equals(&mut self) {
        self.commit();
        let entered = self.operand || self.function.is_some();
        let (a, a_text, op, b, b_text) = match &self.pending {
            // "5 + =" uses the first operand again.
            Some(p) if entered => (
                p.value,
                p.text.clone(),
                p.op,
                self.value,
                self.operand_text(),
            ),
            Some(p) => (p.value, p.text.clone(), p.op, p.value, p.text.clone()),
            None => match self.repeat {
                Some((op, b)) => (self.value, self.operand_text(), op, b, b.display(true)),
                None => {
                    self.expression = format!("{} =", self.operand_text());
                    self.function = None;
                    self.evaluated = true;
                    return;
                }
            },
        };
        let expression = format!("{} {} {} =", a_text, op.symbol(), b_text);
        match op.apply(a, b) {
            Ok(r) => {
                self.value = r;
                self.expression = expression.clone();
                if self.history.len() == MAX_ITEMS {
                    self.history.pop();
                }
                self.history.insert(
                    0,
                    HistoryItem {
                        expression,
                        result: r,
                    },
                );
                self.repeat = Some((op, b));
                self.pending = None;
                self.function = None;
                self.operand = false;
                self.evaluated = true;
            }
            Err(f) => self.fail(f, expression),
        }
    }
    fn percent(&mut self) {
        self.commit();
        let v = match &self.pending {
            Some(Pending {
                value: a,
                op: Op::Add | Op::Sub,
                ..
            }) => a
                .try_mul(self.value)
                .and_then(|p| p.try_div(Number::int(100))),
            Some(Pending {
                op: Op::Mul | Op::Div,
                ..
            }) => self.value.try_div(Number::int(100)),
            None => Ok(Number::ZERO),
        };
        match v {
            Ok(v) => {
                self.value = v;
                self.function = Some(v.display(true));
                self.operand = true;
                self.expression = format!("{}{}", self.prefix(), v.display(true));
            }
            Err(f) => self.fail(f, self.expression.clone()),
        }
    }
    fn negate(&mut self) {
        if let Some(t) = &mut self.typing {
            if t == "0" {
                return;
            }
            if let Some(rest) = t.strip_prefix('-') {
                *t = rest.into();
            } else {
                t.insert(0, '-');
            }
            return;
        }
        let inner = self.operand_text();
        self.value = self.value.negated();
        self.function = Some(format!("negate({inner})"));
        self.after_function();
    }
    fn function(&mut self, name: &str, f: fn(Number) -> Result<Number, Fault>) {
        self.commit();
        let inner = self.operand_text();
        let text = format!("{name}({inner})");
        match f(self.value) {
            Ok(v) => {
                self.value = v;
                self.function = Some(text);
                self.after_function();
            }
            Err(fault) => {
                let e = format!("{}{}", self.prefix(), text);
                self.fail(fault, e);
            }
        }
    }
    fn after_function(&mut self) {
        self.operand = true;
        self.evaluated = false;
        self.expression = format!(
            "{}{}",
            self.prefix(),
            self.function.as_deref().unwrap_or("")
        );
    }
    /// Recalls a history item: its result and expression.
    pub fn recall(&mut self, index: usize) {
        if let Some(item) = self.history.get(index).cloned() {
            self.reset();
            self.value = item.result;
            self.expression = item.expression;
            self.evaluated = true;
        }
    }
    /// Memory item `index` recalled, cleared, added to or subtracted from
    /// (the buttons a memory item shows on hover).
    pub fn memory_item(&mut self, index: usize, action: Command) {
        if index >= self.memory.len() || self.error() {
            return;
        }
        self.commit();
        match action {
            Command::MemoryRecall => {
                self.value = self.memory[index];
                self.function = None;
                self.operand = true;
            }
            Command::MemoryClear => {
                self.memory.remove(index);
            }
            Command::MemoryAdd | Command::MemorySubtract => {
                let v = if action == Command::MemoryAdd {
                    self.value
                } else {
                    self.value.negated()
                };
                if let Ok(sum) = self.memory[index].try_add(v) {
                    self.memory[index] = sum;
                }
            }
            _ => (),
        }
        self.end_entry();
    }
    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}

/// A typed entry with thousands separators ("1234.50" -> "1,234.50").
fn typed_display(t: &str) -> String {
    let (neg, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t),
    };
    let (int, frac) = match body.find('.') {
        Some(i) => (&body[..i], Some(&body[i..])),
        None => (body, None),
    };
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&group(int));
    if let Some(f) = frac {
        s.push_str(f);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use Command::*;
    fn run(keys: &str) -> Calculator {
        let mut c = Calculator::new();
        for k in keys.split(' ') {
            c.press(match k {
                "+" => Op(super::Op::Add),
                "-" => Op(super::Op::Sub),
                "*" => Op(super::Op::Mul),
                "/" => Op(super::Op::Div),
                "=" => Equals,
                "." => Point,
                "%" => Percent,
                "CE" => ClearEntry,
                "C" => Clear,
                "BS" => Backspace,
                "neg" => Negate,
                "inv" => Reciprocal,
                "sqr" => Square,
                "sqrt" => SquareRoot,
                "MS" => MemoryStore,
                "MR" => MemoryRecall,
                "M+" => MemoryAdd,
                "M-" => MemorySubtract,
                "MC" => MemoryClear,
                d => {
                    for b in d.bytes() {
                        c.press(Digit(b - b'0'));
                    }
                    continue;
                }
            });
        }
        c
    }
    #[test]
    fn immediate_execution_and_the_expression_line() {
        let c = run("2 + 3 * 4 =");
        assert_eq!(c.display(), "20");
        assert_eq!(c.expression(), "5 \u{d7} 4 =");
        let c = run("2 + 3 *");
        assert_eq!((c.display().as_str(), c.expression()), ("5", "5 \u{d7} "));
        let c = run("12 + 7");
        assert_eq!((c.display().as_str(), c.expression()), ("7", "12 + "));
        assert_eq!(run("1234567").display(), "1,234,567");
        assert_eq!(run("0 . 5 0").display(), "0.50");
    }
    #[test]
    fn equals_repeats_and_reuses_the_first_operand() {
        assert_eq!(run("5 + = =").display(), "15");
        assert_eq!(run("10 - 2 = = =").display(), "4");
        let c = run("10 - 2 = = =");
        assert_eq!(c.expression(), "6 \u{2212} 2 =");
        assert_eq!(c.history.len(), 3);
        assert_eq!(c.history[0].expression, "6 \u{2212} 2 =");
    }
    #[test]
    fn percent_functions_and_negate_write_windows_expressions() {
        let c = run("50 + 10 %");
        assert_eq!((c.display().as_str(), c.expression()), ("5", "50 + 5"));
        assert_eq!(run("50 + 10 % =").display(), "55");
        assert_eq!(run("200 * 10 % =").display(), "20");
        let c = run("3 sqr");
        assert_eq!((c.display().as_str(), c.expression()), ("9", "sqr(3)"));
        let c = run("3 sqr sqr");
        assert_eq!(c.expression(), "sqr(sqr(3))");
        let c = run("9 sqrt + 1 =");
        assert_eq!(
            (c.display().as_str(), c.expression()),
            ("4", "\u{221a}(9) + 1 =")
        );
        let c = run("5 = neg");
        assert_eq!((c.display().as_str(), c.expression()), ("-5", "negate(5)"));
        assert_eq!(run("12 neg").display(), "-12");
        assert_eq!(run("4 inv").display(), "0.25");
    }
    #[test]
    fn errors_disable_operators_until_cleared() {
        let c = run("1 / 0 =");
        assert_eq!(c.display(), "Cannot divide by zero");
        assert_eq!(c.expression(), "1 \u{f7} 0 =");
        assert!(!c.enabled(Op(super::Op::Add)) && c.enabled(Digit(1)));
        let c = run("1 / 0 = 7");
        assert_eq!((c.display().as_str(), c.expression()), ("7", ""));
        assert_eq!(run("0 inv").display(), "Cannot divide by zero");
        assert_eq!(run("4 neg sqrt").display(), "Invalid input");
        assert_eq!(run("0 / 0 =").display(), "Invalid input");
    }
    #[test]
    fn editing_memory_and_history() {
        assert_eq!(run("123 BS").display(), "12");
        assert_eq!(run("5 BS").display(), "0");
        assert_eq!(run("12 + 3 CE 4 =").display(), "16");
        assert_eq!(run("12 + 3 C").display(), "0");
        let c = run("7 MS 3 M+ 1 M- C MR");
        assert_eq!(c.display(), "9");
        assert_eq!(c.memory.len(), 1);
        let c = run("7 MS 2 MS");
        assert_eq!(c.memory.len(), 2);
        let mut c = run("2 + 2 = 3 * 3 =");
        assert_eq!(c.history[1].result.display(true), "4");
        c.recall(1);
        assert_eq!((c.display().as_str(), c.expression()), ("4", "2 + 2 ="));
        assert!(!run("1").enabled(MemoryRecall));
    }
}
