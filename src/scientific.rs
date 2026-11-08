use std::f64::consts::{E, FRAC_PI_2, PI, TAU};

use serde::Deserialize;

use crate::CalcError;

const MAX_EXPRESSION_BYTES: usize = 4096;
const MAX_NESTING: usize = 64;

/// Controls the input and output units of circular trigonometric functions.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AngleMode {
    #[default]
    Radians,
    Degrees,
}

impl AngleMode {
    fn to_radians(self, angle: f64) -> f64 {
        match self {
            Self::Radians => angle,
            Self::Degrees => (angle % 360.0).to_radians(),
        }
    }

    fn output_angle(self, radians: f64) -> f64 {
        match self {
            Self::Radians => radians,
            Self::Degrees => radians.to_degrees(),
        }
    }

    fn is_tangent_pole(self, angle: f64) -> bool {
        match self {
            Self::Radians => angle.rem_euclid(PI) == FRAC_PI_2,
            Self::Degrees => angle.rem_euclid(180.0) == 90.0,
        }
    }
}

/// Evaluates a scientific expression with finite, real-valued arithmetic.
pub fn evaluate(expression: &str, angle_mode: AngleMode) -> Result<f64, CalcError> {
    if expression.len() > MAX_EXPRESSION_BYTES {
        return Err(CalcError::InvalidInput(format!(
            "Expression is longer than {MAX_EXPRESSION_BYTES} bytes"
        )));
    }

    let mut parser = Parser {
        expression,
        position: 0,
        angle_mode,
    };
    let result = parser.sum(0)?;
    if parser.peek().is_some() {
        return Err(parser.syntax_error("Unexpected input"));
    }
    Ok(result)
}

struct Parser<'a> {
    expression: &'a str,
    position: usize,
    angle_mode: AngleMode,
}

impl Parser<'_> {
    fn sum(&mut self, depth: usize) -> Result<f64, CalcError> {
        let mut value = self.product(depth)?;
        loop {
            value = match self.peek() {
                Some(b'+') => {
                    self.position += 1;
                    checked(value + self.product(depth)?, "Addition")?
                }
                Some(b'-') => {
                    self.position += 1;
                    checked(value - self.product(depth)?, "Subtraction")?
                }
                _ => return Ok(value),
            };
        }
    }

    fn product(&mut self, depth: usize) -> Result<f64, CalcError> {
        let mut value = self.unary(depth)?;
        loop {
            value = match self.peek() {
                Some(b'*') => {
                    self.position += 1;
                    checked(value * self.unary(depth)?, "Multiplication")?
                }
                Some(operator @ (b'/' | b'%')) => {
                    self.position += 1;
                    let divisor = self.unary(depth)?;
                    if divisor == 0.0 {
                        return Err(CalcError::Domain("Division by zero".into()));
                    }
                    if operator == b'/' {
                        checked(value / divisor, "Division")?
                    } else {
                        value % divisor
                    }
                }
                _ => return Ok(value),
            };
        }
    }

    fn unary(&mut self, depth: usize) -> Result<f64, CalcError> {
        if depth > MAX_NESTING {
            return Err(self.syntax_error("Expression is nested too deeply"));
        }
        match self.peek() {
            Some(b'+') => {
                self.position += 1;
                self.unary(depth + 1)
            }
            Some(b'-') => {
                self.position += 1;
                Ok(-self.unary(depth + 1)?)
            }
            _ => self.power(depth),
        }
    }

    fn power(&mut self, depth: usize) -> Result<f64, CalcError> {
        let mut base = self.atom(depth)?;
        if self.consume(b'!') {
            base = factorial(base)?;
        }
        if self.consume(b'^') {
            power(base, self.unary(depth + 1)?)
        } else {
            Ok(base)
        }
    }

    fn atom(&mut self, depth: usize) -> Result<f64, CalcError> {
        match self.peek() {
            Some(b'(') => {
                self.position += 1;
                let value = self.sum(depth + 1)?;
                self.expect(b')', "Expected a closing parenthesis")?;
                Ok(value)
            }
            Some(b'0'..=b'9' | b'.') => self.number(),
            Some(b'a'..=b'z' | b'A'..=b'Z') => self.identifier(depth),
            _ => Err(self.syntax_error("Expected a number, constant, function, or parenthesis")),
        }
    }

    fn number(&mut self) -> Result<f64, CalcError> {
        let start = self.position;
        self.digits();
        if self.current() == Some(b'.') {
            self.position += 1;
            self.digits();
        }
        if matches!(self.current(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.current(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            let exponent_start = self.position;
            self.digits();
            if self.position == exponent_start {
                return Err(self.syntax_error("Expected digits in the exponent"));
            }
        }

        let literal = &self.expression[start..self.position];
        let value = literal
            .parse()
            .map_err(|_| CalcError::InvalidInput(format!("Invalid number at byte {start}")))?;
        checked(value, "Number")
    }

    fn identifier(&mut self, depth: usize) -> Result<f64, CalcError> {
        let start = self.position;
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        {
            self.position += 1;
        }
        let name = self.expression[start..self.position].to_ascii_lowercase();
        if !self.consume(b'(') {
            return match name.as_str() {
                "pi" => Ok(PI),
                "tau" => Ok(TAU),
                "e" => Ok(E),
                _ => Err(CalcError::InvalidInput(format!(
                    "Unknown constant or missing function parentheses at byte {start}: {name}"
                ))),
            };
        }

        let first = self.sum(depth + 1)?;
        let second = if self.consume(b',') {
            Some(self.sum(depth + 1)?)
        } else {
            None
        };
        self.expect(
            b')',
            "Expected a closing parenthesis after function arguments",
        )?;
        apply_function(&name, first, second, self.angle_mode)
    }

    fn digits(&mut self) {
        while matches!(self.current(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
    }

    fn current(&self) -> Option<u8> {
        self.expression.as_bytes().get(self.position).copied()
    }

    fn peek(&mut self) -> Option<u8> {
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.position += 1;
        }
        self.current()
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8, message: &str) -> Result<(), CalcError> {
        if self.consume(byte) {
            Ok(())
        } else {
            Err(self.syntax_error(message))
        }
    }

    fn syntax_error(&self, message: &str) -> CalcError {
        CalcError::InvalidInput(format!("{message} at byte {}", self.position))
    }
}

fn checked(value: f64, operation: &str) -> Result<f64, CalcError> {
    if value.is_nan() {
        Err(CalcError::Domain(format!(
            "{operation} is undefined for the supplied input"
        )))
    } else if value.is_infinite() {
        Err(CalcError::Numerical(format!(
            "{operation} exceeds the floating-point range"
        )))
    } else {
        Ok(value)
    }
}

fn factorial(value: f64) -> Result<f64, CalcError> {
    if value < 0.0 || value.fract() != 0.0 {
        return Err(CalcError::Domain(
            "Factorial requires a nonnegative integer".into(),
        ));
    }
    if value > 170.0 {
        return Err(CalcError::Numerical(
            "Factorial exceeds the floating-point range above 170".into(),
        ));
    }
    Ok((2..=value as u32).map(f64::from).product())
}

fn power(base: f64, exponent: f64) -> Result<f64, CalcError> {
    if base == 0.0 && exponent < 0.0 {
        return Err(CalcError::Domain("Zero raised to a negative power".into()));
    }
    checked(base.powf(exponent), "Power")
}

fn apply_function(
    name: &str,
    first: f64,
    second: Option<f64>,
    mode: AngleMode,
) -> Result<f64, CalcError> {
    let value = match (name, second) {
        ("sin", None) => mode.to_radians(first).sin(),
        ("cos", None) => mode.to_radians(first).cos(),
        ("tan", None) if mode.is_tangent_pole(first) => {
            return Err(CalcError::Domain(
                "Tangent is undefined at this angle".into(),
            ));
        }
        ("tan", None) => mode.to_radians(first).tan(),
        ("asin", None) => mode.output_angle(first.asin()),
        ("acos", None) => mode.output_angle(first.acos()),
        ("atan", None) => mode.output_angle(first.atan()),
        ("atan2", Some(second)) if first != 0.0 || second != 0.0 => {
            mode.output_angle(first.atan2(second))
        }
        ("atan2", Some(_)) => {
            return Err(CalcError::Domain("atan2 is undefined at the origin".into()));
        }
        ("sinh", None) => first.sinh(),
        ("cosh", None) => first.cosh(),
        ("tanh", None) => first.tanh(),
        ("sqrt", None) => first.sqrt(),
        ("cbrt", None) => first.cbrt(),
        ("abs", None) => first.abs(),
        ("exp", None) => first.exp(),
        ("ln" | "log" | "log10" | "log2", None) if first <= 0.0 => {
            return Err(CalcError::Domain(
                "Logarithms require positive input".into(),
            ));
        }
        ("ln", None) => first.ln(),
        ("log" | "log10", None) => first.log10(),
        ("log2", None) => first.log2(),
        ("log", Some(base)) => {
            if first <= 0.0 || base <= 0.0 || base == 1.0 {
                return Err(CalcError::Domain(
                    "Logarithms require positive input and a positive base other than 1".into(),
                ));
            }
            first.log(base)
        }
        ("floor", None) => first.floor(),
        ("ceil", None) => first.ceil(),
        ("round", None) => first.round(),
        ("trunc", None) => first.trunc(),
        ("min", Some(second)) => first.min(second),
        ("max", Some(second)) => first.max(second),
        ("hypot", Some(second)) => first.hypot(second),
        ("pow", Some(second)) => return power(first, second),
        _ => {
            return Err(CalcError::InvalidInput(format!(
                "Unknown function or incorrect number of arguments: {name}"
            )));
        }
    };
    checked(value, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_operator_precedence_and_associativity() {
        for (expression, expected) in [
            ("2 + 3 * (8 - 4)", 14.0),
            ("2^3^2", 512.0),
            ("-2^2", -4.0),
            ("(-2)^2", 4.0),
            ("2^-3", 0.125),
            ("2^3!", 64.0),
            ("0! + 5!", 121.0),
            ("-8 % 3", -2.0),
            (".5E+2 - 4.2e-1", 49.58),
        ] {
            let actual = evaluate(expression, AngleMode::Radians).unwrap();
            assert!((actual - expected).abs() < 1e-12, "{expression}");
        }
    }

    #[test]
    fn evaluates_functions_with_angle_modes() {
        for (expression, mode, expected) in [
            ("sin(pi/2)", AngleMode::Radians, 1.0),
            ("Sin(90) + cos(180)", AngleMode::Degrees, 0.0),
            ("asin(0.5)", AngleMode::Degrees, 30.0),
            ("atan2(1, -1)", AngleMode::Degrees, 135.0),
            ("log(64, 2) + sqrt(16)", AngleMode::Radians, 10.0),
            ("hypot(3, 4) + ln(e)", AngleMode::Radians, 6.0),
        ] {
            let actual = evaluate(expression, mode).unwrap();
            assert!((actual - expected).abs() < 1e-12, "{expression}");
        }
    }

    #[test]
    fn distinguishes_syntax_domain_and_range_errors() {
        for expression in ["", "1e+", "sin()", "max(1)", "2 pi", "(1", "3!!", "π"] {
            assert!(
                matches!(
                    evaluate(expression, AngleMode::Radians),
                    Err(CalcError::InvalidInput(_))
                ),
                "{expression}"
            );
        }
        for expression in ["1 / 0", "(-2)!", "0^-1", "sqrt(-1)", "tan(pi/2)", "ln(0)"] {
            assert!(
                matches!(
                    evaluate(expression, AngleMode::Radians),
                    Err(CalcError::Domain(_))
                ),
                "{expression}"
            );
        }
        for expression in ["1e999", "171!", "1e300 * 1e300"] {
            assert!(
                matches!(
                    evaluate(expression, AngleMode::Radians),
                    Err(CalcError::Numerical(_))
                ),
                "{expression}"
            );
        }
    }

    #[test]
    fn bounds_expression_size_and_recursion() {
        for expression in [
            " ".repeat(MAX_EXPRESSION_BYTES + 1),
            format!("{}1", "-".repeat(MAX_NESTING + 1)),
            format!("{}1", "1^".repeat(MAX_NESTING + 1)),
            format!(
                "{}1{}",
                "(".repeat(MAX_NESTING + 1),
                ")".repeat(MAX_NESTING + 1)
            ),
        ] {
            assert!(matches!(
                evaluate(&expression, AngleMode::Radians),
                Err(CalcError::InvalidInput(_))
            ));
        }
    }
}
