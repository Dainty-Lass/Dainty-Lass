//! The two decimal rules (R1 2).

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// Leading whitespace, an optional sign and digits; the value in `i64` and where the digits ended.
fn leading(bytes: &[u8]) -> Option<(i64, usize)> {
    let mut at = bytes.iter().take_while(|&&byte| is_space(byte)).count();
    let negative = match bytes.get(at) {
        Some(b'-') => {
            at += 1;
            true
        }
        Some(b'+') => {
            at += 1;
            false
        }
        _ => false,
    };
    let digits = bytes[at..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let mut value: i64 = 0;
    for &digit in &bytes[at..at + digits] {
        value = value.checked_mul(10)?.checked_sub(i64::from(digit - b'0'))?;
    }
    let value = if negative { value } else { value.checked_neg()? };
    Some((value, at + digits))
}

/// A `strtol` integer: optional whitespace, an optional sign, digits and nothing after; outside `i32` is `None`.
pub fn strtol(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    let (value, end) = leading(bytes)?;
    if end != bytes.len() {
        return None;
    }
    i32::try_from(value).ok()
}

/// A stream integer as `i64`: whitespace, sign, digits, the rest ignored. A BOM is not whitespace.
pub fn stream_i64(bytes: &[u8]) -> Option<i64> {
    leading(bytes).map(|(value, _)| value)
}

/// A stream integer as `i32`.
pub fn stream_i32(bytes: &[u8]) -> Option<i32> {
    stream_i64(bytes).and_then(|value| i32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strtol_rule() {
        assert_eq!(strtol("30"), Some(30));
        assert_eq!(strtol(" \t30"), Some(30));
        assert_eq!(strtol("+30"), Some(30));
        assert_eq!(strtol("-0"), Some(0));
        assert_eq!(strtol("-12"), Some(-12));
        assert_eq!(strtol("30 "), None);
        assert_eq!(strtol("12x"), None);
        assert_eq!(strtol(""), None);
        assert_eq!(strtol("-"), None);
        assert_eq!(strtol("2147483647"), Some(i32::MAX));
        assert_eq!(strtol("-2147483648"), Some(i32::MIN));
        assert_eq!(strtol("2147483648"), None);
        assert_eq!(strtol("99999999999999999999999"), None);
    }

    #[test]
    fn stream_rule() {
        assert_eq!(stream_i32(b" 12\nx"), Some(12));
        assert_eq!(stream_i32(b"12x"), Some(12));
        assert_eq!(stream_i32(b"x12"), None);
        assert_eq!(stream_i32(b""), None);
        assert_eq!(stream_i32(b"\xEF\xBB\xBF12"), None);
        assert_eq!(stream_i32(b"99999999999"), None);
        assert_eq!(stream_i64(b"99999999999"), Some(99_999_999_999));
        assert_eq!(stream_i64(b"-5 seconds"), Some(-5));
        assert_eq!(stream_i64(b"9223372036854775808"), None);
        assert_eq!(stream_i64(b"-9223372036854775808"), Some(i64::MIN));
    }
}
