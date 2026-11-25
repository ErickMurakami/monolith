#[derive(Debug, PartialEq, Eq)]
pub enum CalculationError {
    Syntax,
    DivisionByZero,
    OutOfRange,
}

pub fn calculate(expression: &str) -> Result<f64, CalculationError> {
    if expression.len() > 256 {
        return Err(CalculationError::OutOfRange);
    }
    let mut remaining = expression.trim_ascii().as_bytes();
    let value = expression_value(&mut remaining)?;
    if remaining.is_empty() {
        Ok(value)
    } else {
        Err(CalculationError::Syntax)
    }
}

fn finite(value: f64) -> Result<f64, CalculationError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CalculationError::OutOfRange)
    }
}

fn consume(input: &mut &[u8], symbols: &[u8]) -> Option<u8> {
    *input = input.trim_ascii_start();
    let (symbol, rest) = input.split_first()?;
    if symbols.contains(symbol) {
        *input = rest;
        Some(*symbol)
    } else {
        None
    }
}

fn expression_value(input: &mut &[u8]) -> Result<f64, CalculationError> {
    let mut value = product(input)?;
    while let Some(operator) = consume(input, b"+-") {
        let rhs = product(input)?;
        value = finite(if operator == b'+' {
            value + rhs
        } else {
            value - rhs
        })?;
    }
    Ok(value)
}

fn product(input: &mut &[u8]) -> Result<f64, CalculationError> {
    let mut value = operand(input)?;
    while let Some(operator) = consume(input, b"*/") {
        let rhs = operand(input)?;
        value = if operator == b'*' {
            finite(value * rhs)?
        } else if rhs == 0.0 {
            return Err(CalculationError::DivisionByZero);
        } else {
            finite(value / rhs)?
        };
    }
    Ok(value)
}

fn operand(input: &mut &[u8]) -> Result<f64, CalculationError> {
    let sign = if consume(input, b"+-") == Some(b'-') {
        -1.0
    } else {
        1.0
    };
    if consume(input, b"(").is_some() {
        let value = expression_value(input)?;
        return if consume(input, b")").is_some() {
            Ok(sign * value)
        } else {
            Err(CalculationError::Syntax)
        };
    }
    let len = input
        .iter()
        .take_while(|c| c.is_ascii_digit() || **c == b'.')
        .count();
    let (literal, rest) = input.split_at(len);
    let value: f64 = std::str::from_utf8(literal)
        .map_err(|_| CalculationError::Syntax)?
        .parse()
        .map_err(|_| CalculationError::Syntax)?;
    *input = rest;
    finite(sign * value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_with_precedence_and_decimal_operands() {
        assert_eq!(calculate("2 + 3 * 4"), Ok(14.0));
        assert_eq!(calculate(" -(2 + 3) / .5 "), Ok(-10.0));
        assert_eq!(calculate("8 / 4 / 2"), Ok(1.0));
    }

    #[test]
    fn rejects_incomplete_invalid_and_unbounded_expressions() {
        assert_eq!(
            calculate("3 / (1 - 1)"),
            Err(CalculationError::DivisionByZero)
        );
        for expression in ["", "()", "(2", "2 3", "1.2.3", "1 +", "sqrt(2)", "1,5"] {
            assert_eq!(calculate(expression), Err(CalculationError::Syntax));
        }
        assert_eq!(
            calculate(&"1".repeat(257)),
            Err(CalculationError::OutOfRange)
        );
    }
}
