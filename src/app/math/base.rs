use super::expr::MathError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseConversion {
    pub source_base: u32,
    pub value: i128,
    pub rows: Vec<BaseRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseRow {
    pub base: u32,
    pub value: String,
}

pub fn convert_base(input: &str) -> Result<BaseConversion, MathError> {
    let words = input.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return Err(MathError::new(
            "usage: :base <value> [from <2..16>] to <2..16|2..16|all>",
        ));
    }

    let value_literal = words[0];
    let mut source_base = infer_source_base(value_literal);
    let mut targets: Option<Vec<u32>> = None;

    let mut idx = 1;
    while idx < words.len() {
        match words[idx].to_ascii_lowercase().as_str() {
            "from" => {
                idx += 1;
                let Some(base) = words.get(idx) else {
                    return Err(MathError::new("expected source base after 'from'"));
                };
                source_base = parse_base(base)?;
                idx += 1;
            }
            "to" => {
                idx += 1;
                let Some(target) = words.get(idx) else {
                    return Err(MathError::new("expected target base after 'to'"));
                };
                targets = Some(parse_targets(target)?);
                idx += 1;
            }
            other => {
                return Err(MathError::new(format!(
                    "unexpected token '{other}'; expected 'from' or 'to'"
                )));
            }
        }
    }

    let value = parse_value(value_literal, source_base)?;
    let targets = targets.unwrap_or_else(|| (2..=16).collect());
    let rows = targets
        .into_iter()
        .map(|base| BaseRow {
            base,
            value: format_value(value, base),
        })
        .collect();

    Ok(BaseConversion {
        source_base,
        value,
        rows,
    })
}

fn parse_targets(input: &str) -> Result<Vec<u32>, MathError> {
    if input.eq_ignore_ascii_case("all") {
        return Ok((2..=16).collect());
    }

    if let Some((start, end)) = input.split_once("..") {
        let start = parse_base(start)?;
        let end = parse_base(end)?;
        if start > end {
            return Err(MathError::new("base range must be ascending"));
        }
        return Ok((start..=end).collect());
    }

    Ok(vec![parse_base(input)?])
}

fn parse_base(input: &str) -> Result<u32, MathError> {
    let base = input
        .parse::<u32>()
        .map_err(|_| MathError::new(format!("invalid base '{input}'")))?;
    if !(2..=16).contains(&base) {
        return Err(MathError::new(format!(
            "invalid base {base}; supported range is 2..16"
        )));
    }
    Ok(base)
}

fn infer_source_base(input: &str) -> u32 {
    let unsigned = input.trim_start_matches(['+', '-']);
    if unsigned.starts_with("0b") || unsigned.starts_with("0B") {
        2
    } else if unsigned.starts_with("0o") || unsigned.starts_with("0O") {
        8
    } else if unsigned.starts_with("0x") || unsigned.starts_with("0X") {
        16
    } else {
        10
    }
}

fn parse_value(input: &str, base: u32) -> Result<i128, MathError> {
    let mut chars = input.trim().chars().peekable();
    let negative = match chars.peek().copied() {
        Some('-') => {
            chars.next();
            true
        }
        Some('+') => {
            chars.next();
            false
        }
        _ => false,
    };

    let mut body = chars.collect::<String>();
    if matches!(base, 2 | 8 | 16) {
        let lower = body.to_ascii_lowercase();
        let prefix = match base {
            2 => "0b",
            8 => "0o",
            16 => "0x",
            _ => unreachable!(),
        };
        if lower.starts_with(prefix) {
            body = body[2..].to_string();
        }
    }

    let compact = body.replace('_', "");
    if compact.is_empty() {
        return Err(MathError::new("empty numeric value"));
    }

    let mut value: i128 = 0;
    for ch in compact.chars() {
        let Some(digit) = ch.to_digit(16) else {
            return Err(MathError::new(format!("invalid digit '{ch}'")));
        };
        if digit >= base {
            return Err(MathError::new(format!(
                "digit '{ch}' is invalid for base {base}"
            )));
        }
        value = value
            .checked_mul(base as i128)
            .and_then(|current| current.checked_add(digit as i128))
            .ok_or_else(|| MathError::new("integer overflow while parsing value"))?;
    }

    if negative {
        value
            .checked_neg()
            .ok_or_else(|| MathError::new("integer overflow while applying sign"))
    } else {
        Ok(value)
    }
}

fn format_value(value: i128, base: u32) -> String {
    debug_assert!((2..=16).contains(&base));
    if value == 0 {
        return "0".to_string();
    }

    let negative = value < 0;
    let mut magnitude = value.unsigned_abs();
    let mut digits = Vec::new();
    while magnitude > 0 {
        let digit = (magnitude % base as u128) as u8;
        digits.push(match digit {
            0..=9 => (b'0' + digit) as char,
            10..=15 => (b'A' + digit - 10) as char,
            _ => unreachable!(),
        });
        magnitude /= base as u128;
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_between_every_supported_base() {
        let conversion = convert_base("255 from 10 to 2..16").unwrap();
        assert_eq!(conversion.rows.len(), 15);
        assert_eq!(conversion.rows[0].value, "11111111");
        assert_eq!(conversion.rows[14].value, "FF");
    }

    #[test]
    fn validates_digits_against_source_base() {
        let err = convert_base("102 from 2 to 10").unwrap_err();
        assert!(err.message.contains("invalid for base 2"));
    }
}
