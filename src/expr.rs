//! 表达式求值器（无第三方依赖）：数学 + 比较 + 逻辑运算。
//!
//! - 数学运算符：`+` `-` `*` `/` `%`（取余）`^`（幂，右结合）
//! - 比较运算符（仅逻辑模式）：`==` `!=` `<` `<=` `>` `>=`（结果为 1.0/0.0）
//! - 逻辑运算符（仅逻辑模式）：`&&`（与）`||`（或）`!`（非），非 0 视为真
//! - 函数：`sin cos tan asin acos atan atan2 sqrt cbrt abs floor ceil round sign
//!   min max clamp pow exp ln/log log10 fmod lerp pi`
//! - 范围函数：`in_range(v,min,max)`（含端点）、`in_range_exclusively(v,min,max)`（不含端点）
//! - 变量：`$name` 或 `name`（不带 `$` 也行），经 `resolve` 回调返回数值；
//!   材质代理里用它读取已声明的 VMT 变量（未定义变量解析为 0.0）。
//!
//! `l4nrp_math` 用 [`eval_math`]（仅数学，不含比较/逻辑）；`l4nrp_logic` 用 [`eval_logic`]
//! （数学 + 比较 + 逻辑）。

/// 表达式错误。
pub struct ExprError(pub String);

#[derive(Clone, Copy)]
enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// 非 0 视为逻辑真。
#[inline]
fn truth(x: f32) -> bool {
    x != 0.0
}

/// 求值（数学模式：仅四则/幂/括号/函数，**不支持**比较与逻辑运算符）。供 `l4nrp_math` 使用。
///
/// 例：`"($v_1 * 2) + min($v_2, 3) ^ 2"`
pub fn eval_math(input: &str, resolve: &mut dyn FnMut(&str) -> f32) -> Result<f32, ExprError> {
    eval_with(input, resolve, false)
}

/// 求值（逻辑模式：在数学基础上支持比较 `== != < <= > >=` 与逻辑 `&& || !`，
/// 以及 `in_range` / `in_range_exclusively`）。供 `l4nrp_logic` 使用。
///
/// 例：`"$a >= 1 && $b < 3 || !$c"`
/// 例：`"in_range($v, $min, $max) && !$off"`
pub fn eval_logic(input: &str, resolve: &mut dyn FnMut(&str) -> f32) -> Result<f32, ExprError> {
    eval_with(input, resolve, true)
}

fn eval_with(
    input: &str,
    resolve: &mut dyn FnMut(&str) -> f32,
    logical: bool,
) -> Result<f32, ExprError> {
    let mut p = Parser {
        chars: input.chars().peekable(),
        resolve,
        logical,
    };
    let v = if logical {
        p.parse_logic_or()?
    } else {
        p.parse_expr()?
    };
    if let Some(c) = p.peek() {
        return Err(ExprError(format!("unexpected '{c}'")));
    }
    Ok(v)
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    resolve: &'a mut dyn FnMut(&str) -> f32,
    logical: bool,
}

impl Parser<'_> {
    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.chars.peek().copied()
    }
    fn next(&mut self) -> Option<char> {
        self.skip_ws();
        self.chars.next()
    }
    fn skip_ws(&mut self) {
        while matches!(self.chars.peek(), Some(c) if c.is_whitespace()) {
            self.chars.next();
        }
    }
    /// 尝试消费两字符运算符 `ab`（中间不允许空白）。成功则消费并返回 true。
    fn two_char(&mut self, a: char, b: char) -> bool {
        let mut it = self.chars.clone();
        if it.next() == Some(a) && it.next() == Some(b) {
            self.chars.next();
            self.chars.next();
            true
        } else {
            false
        }
    }

    // logic_or := logic_and (('||') logic_and)*   （仅逻辑模式）
    fn parse_logic_or(&mut self) -> Result<f32, ExprError> {
        let mut v = self.parse_logic_and()?;
        while self.logical {
            self.skip_ws();
            if self.two_char('|', '|') {
                let r = self.parse_logic_and()?;
                v = if truth(v) || truth(r) { 1.0 } else { 0.0 };
            } else {
                break;
            }
        }
        Ok(v)
    }

    // logic_and := compare (('&&') compare)*   （仅逻辑模式）
    fn parse_logic_and(&mut self) -> Result<f32, ExprError> {
        let mut v = self.parse_compare()?;
        while self.logical {
            self.skip_ws();
            if self.two_char('&', '&') {
                let r = self.parse_compare()?;
                v = if truth(v) && truth(r) { 1.0 } else { 0.0 };
            } else {
                break;
            }
        }
        Ok(v)
    }

    // compare := expr (('=='|'!='|'<='|'>='|'<'|'>') expr)?   （仅逻辑模式）
    fn parse_compare(&mut self) -> Result<f32, ExprError> {
        let l = self.parse_expr()?;
        if !self.logical {
            return Ok(l);
        }
        self.skip_ws();
        let op: Option<CmpOp> = if self.two_char('=', '=') {
            Some(CmpOp::Eq)
        } else if self.two_char('!', '=') {
            Some(CmpOp::Ne)
        } else if self.two_char('<', '=') {
            Some(CmpOp::Le)
        } else if self.two_char('>', '=') {
            Some(CmpOp::Ge)
        } else {
            match self.peek() {
                Some('<') => {
                    self.chars.next();
                    Some(CmpOp::Lt)
                }
                Some('>') => {
                    self.chars.next();
                    Some(CmpOp::Gt)
                }
                _ => None,
            }
        };
        match op {
            Some(op) => {
                let r = self.parse_expr()?;
                let b = match op {
                    CmpOp::Eq => l == r,
                    CmpOp::Ne => l != r,
                    CmpOp::Lt => l < r,
                    CmpOp::Le => l <= r,
                    CmpOp::Gt => l > r,
                    CmpOp::Ge => l >= r,
                };
                Ok(if b { 1.0 } else { 0.0 })
            }
            None => Ok(l),
        }
    }

    // expr := term (('+'|'-') term)*
    fn parse_expr(&mut self) -> Result<f32, ExprError> {
        let mut v = self.parse_term()?;
        loop {
            match self.peek() {
                Some('+') => {
                    self.chars.next();
                    v += self.parse_term()?;
                }
                Some('-') => {
                    self.chars.next();
                    v -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    // term := factor (('*'|'/'|'%') factor)*
    fn parse_term(&mut self) -> Result<f32, ExprError> {
        let mut v = self.parse_factor()?;
        loop {
            match self.peek() {
                Some('*') => {
                    self.chars.next();
                    v *= self.parse_factor()?;
                }
                Some('/') => {
                    self.chars.next();
                    let r = self.parse_factor()?;
                    if r == 0.0 {
                        return Err(ExprError("division by zero".into()));
                    }
                    v /= r;
                }
                Some('%') => {
                    self.chars.next();
                    let r = self.parse_factor()?;
                    if r == 0.0 {
                        return Err(ExprError("modulo by zero".into()));
                    }
                    v %= r;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    // factor := unary ('^' factor)?   （幂右结合）
    fn parse_factor(&mut self) -> Result<f32, ExprError> {
        let v = self.parse_unary()?;
        if self.peek() == Some('^') {
            self.chars.next();
            let r = self.parse_factor()?; // 右结合：2^3^2 == 2^(3^2)
            Ok(v.powf(r))
        } else {
            Ok(v)
        }
    }

    // unary := ('-'|'+'|'!')* primary   （'!' 仅逻辑模式）
    fn parse_unary(&mut self) -> Result<f32, ExprError> {
        match self.peek() {
            Some('-') => {
                self.chars.next();
                Ok(-self.parse_unary()?)
            }
            Some('+') => {
                self.chars.next();
                self.parse_unary()
            }
            Some('!') if self.logical => {
                self.chars.next();
                let v = self.parse_unary()?;
                Ok(if truth(v) { 0.0 } else { 1.0 })
            }
            _ => self.parse_primary(),
        }
    }

    // primary := number | ident | ident '(' args ')' | '(' expr ')'
    fn parse_primary(&mut self) -> Result<f32, ExprError> {
        let c = self
            .peek()
            .ok_or_else(|| ExprError("unexpected end of expression".into()))?;
        if c.is_ascii_digit() || c == '.' {
            let mut s = String::new();
            while let Some(c) = self.chars.peek() {
                if c.is_ascii_digit() || *c == '.' {
                    s.push(*c);
                    self.chars.next();
                } else {
                    break;
                }
            }
            s.parse::<f32>()
                .map_err(|_| ExprError(format!("invalid number '{s}'")))
        } else if c == '$' || c.is_ascii_alphabetic() {
            let mut name = String::new();
            while let Some(c) = self.chars.peek() {
                if c.is_ascii_alphanumeric() || *c == '_' || *c == '$' {
                    name.push(*c);
                    self.chars.next();
                } else {
                    break;
                }
            }
            if self.peek() == Some('(') {
                let args = self.parse_args()?;
                self.call_func(&name, args)
            } else {
                let vname = name.trim_start_matches('$');
                Ok((self.resolve)(vname))
            }
        } else if c == '(' {
            self.chars.next();
            // 括号内与所在模式一致（逻辑模式支持逻辑/比较；数学模式退化为纯算术）
            let v = if self.logical {
                self.parse_logic_or()?
            } else {
                self.parse_expr()?
            };
            if self.next() != Some(')') {
                return Err(ExprError("expected ')'".into()));
            }
            Ok(v)
        } else {
            Err(ExprError(format!("unexpected '{c}'")))
        }
    }

    /// 解析括号参数列表（调用方已确认紧跟 `(`）。
    fn parse_args(&mut self) -> Result<Vec<f32>, ExprError> {
        self.chars.next(); // 消费 '('
        if self.peek() == Some(')') {
            self.chars.next();
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        loop {
            args.push(if self.logical {
                self.parse_logic_or()?
            } else {
                self.parse_expr()?
            });
            match self.next() {
                Some(',') => continue,
                Some(')') => break,
                _ => return Err(ExprError("expected ',' or ')' in function call".into())),
            }
        }
        Ok(args)
    }

    fn call_func(&mut self, name: &str, args: Vec<f32>) -> Result<f32, ExprError> {
        use std::f32::consts::PI;
        let err = |n: usize, got: usize| ExprError(format!("{name}() expects {n} arg(s), got {got}"));
        let one = |f: fn(f32) -> f32| -> Result<f32, ExprError> {
            if args.len() != 1 {
                return Err(err(1, args.len()));
            }
            Ok(f(args[0]))
        };
        let two = |f: fn(f32, f32) -> f32| -> Result<f32, ExprError> {
            if args.len() != 2 {
                return Err(err(2, args.len()));
            }
            Ok(f(args[0], args[1]))
        };
        match name.to_ascii_lowercase().as_str() {
            "sin" => one(f32::sin),
            "cos" => one(f32::cos),
            "tan" => one(f32::tan),
            "asin" => one(f32::asin),
            "acos" => one(f32::acos),
            "atan" => one(f32::atan),
            "atan2" => two(f32::atan2),
            "sqrt" => one(f32::sqrt),
            "cbrt" => one(f32::cbrt),
            "abs" => one(f32::abs),
            "floor" => one(f32::floor),
            "ceil" => one(f32::ceil),
            "round" => one(f32::round),
            "sign" => one(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }),
            "min" => two(f32::min),
            "max" => two(f32::max),
            "clamp" => {
                if args.len() != 3 {
                    return Err(err(3, args.len()));
                }
                Ok(args[0].clamp(args[1], args[2]))
            }
            "pow" => two(f32::powf),
            "exp" => one(f32::exp),
            "ln" | "log" => one(f32::ln),
            "log10" => one(f32::log10),
            "fmod" => {
                if args.len() != 2 {
                    return Err(err(2, args.len()));
                }
                if args[1] == 0.0 {
                    return Err(ExprError("fmod by zero".into()));
                }
                Ok(args[0] % args[1])
            }
            "lerp" => {
                if args.len() != 3 {
                    return Err(err(3, args.len()));
                }
                Ok(args[0] + (args[1] - args[0]) * args[2])
            }
            // 范围检测（逻辑代理用）：in_range 含端点，in_range_exclusively 不含端点
            "in_range" => {
                if args.len() != 3 {
                    return Err(err(3, args.len()));
                }
                Ok(if args[0] >= args[1] && args[0] <= args[2] {
                    1.0
                } else {
                    0.0
                })
            }
            "in_range_exclusively" => {
                if args.len() != 3 {
                    return Err(err(3, args.len()));
                }
                Ok(if args[0] > args[1] && args[0] < args[2] {
                    1.0
                } else {
                    0.0
                })
            }
            "pi" => {
                if !args.is_empty() {
                    return Err(err(0, args.len()));
                }
                Ok(PI)
            }
            _ => Err(ExprError(format!("unknown function '{name}'"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{eval_math, eval_logic};

    fn resolve(name: &str) -> f32 {
        match name {
            "a" => 5.0,
            "b" => 3.0,
            "c" => 1.0,
            "v" => 2.0,
            _ => 0.0,
        }
    }
    fn math(input: &str) -> Result<f32, String> {
        let mut rf = |n: &str| resolve(n);
        eval_math(input, &mut rf).map_err(|e| e.0)
    }
    fn logic(input: &str) -> Result<f32, String> {
        let mut rf = |n: &str| resolve(n);
        eval_logic(input, &mut rf).map_err(|e| e.0)
    }

    #[test]
    fn math_arithmetic() {
        assert_eq!(math("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(math("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(math("2 ^ 3 ^ 2").unwrap(), 512.0);
        assert_eq!(math("min($a, $b) + max(1, 2)").unwrap(), 5.0);
    }

    #[test]
    fn math_rejects_logic_operators() {
        assert!(math("$a < $b").is_err());
        assert!(math("1 && 0").is_err());
        assert!(math("1 || 0").is_err());
        assert!(math("!$a").is_err());
        assert!(math("$a == 5").is_err());
    }

    #[test]
    fn logic_compare() {
        assert_eq!(logic("$a > $b").unwrap(), 1.0);
        assert_eq!(logic("$a < $b").unwrap(), 0.0);
        assert_eq!(logic("$a >= 5").unwrap(), 1.0);
        assert_eq!(logic("$a == 5").unwrap(), 1.0);
        assert_eq!(logic("$a != 5").unwrap(), 0.0);
        assert_eq!(logic("($a >= 5) == ($b >= 3)").unwrap(), 1.0);
    }

    #[test]
    fn logic_boolean() {
        assert_eq!(logic("$a > $b && $c != 2").unwrap(), 1.0);
        assert_eq!(logic("$a > $b || 0").unwrap(), 1.0);
        assert_eq!(logic("!$c").unwrap(), 0.0); // c=1 → 非真 → 0
        assert_eq!(logic("!$a").unwrap(), 0.0); // a=5 非 0 → 真 → 非 → 0
    }

    #[test]
    fn logic_in_range() {
        assert_eq!(logic("in_range($v, 0, 3)").unwrap(), 1.0);
        assert_eq!(logic("in_range($v, 3, 4)").unwrap(), 0.0);
        assert_eq!(logic("in_range_exclusively($v, 0, 2)").unwrap(), 0.0); // 2 不在 (0,2)
        assert_eq!(logic("in_range_exclusively($v, 0, 3)").unwrap(), 1.0);
        assert_eq!(logic("in_range($v, $min, 3) && !$off").unwrap(), 1.0); // 未定义按 0
    }
}
