//! 数学表达式求值器（无第三方依赖）：支持四则运算、幂、括号、一元负号、常用函数，
//! 以及材质 VMT 变量引用。
//!
//! - 运算符：`+` `-` `*` `/` `%`（取余）`^`（幂，右结合）
//! - 函数：`sin cos tan asin acos atan atan2 sqrt cbrt abs floor ceil round sign
//!   min max clamp pow exp ln/log log10 fmod lerp pi`
//! - 变量：`$name` 或 `name`（不带 `$` 也行），经 `resolve` 回调返回数值；
//!   材质代理里用它读取已声明的 VMT 变量（未定义变量解析为 0.0）。

/// 表达式错误。
pub struct ExprError(pub String);

/// 求值 `input`；`resolve(name)` 把变量名（不含 `$`）解析为 `f32`。
///
/// 例：`"($v_1 * 2) + min($v_2, 3) ^ 2"`
pub fn eval(input: &str, resolve: &mut dyn FnMut(&str) -> f32) -> Result<f32, ExprError> {
    let mut p = Parser {
        chars: input.chars().peekable(),
        resolve,
    };
    let v = p.parse_expr()?;
    if let Some(c) = p.peek() {
        return Err(ExprError(format!("unexpected '{c}'")));
    }
    Ok(v)
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    resolve: &'a mut dyn FnMut(&str) -> f32,
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

    // unary := ('-'|'+')* primary
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
            let v = self.parse_expr()?;
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
            args.push(self.parse_expr()?);
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
