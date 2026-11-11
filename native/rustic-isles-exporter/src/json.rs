//! The output layer's bytes (spec §5): compact JSON with the exporter's escaping, C's `%f`, and CRLF line ends.

/// Compact JSON: no spaces anywhere; commas placed as values are added.
#[derive(Default)]
pub struct Json {
    pub buf: Vec<u8>,
    /// One per open object or array: whether it has a value yet.
    open: Vec<bool>,
    after_key: bool,
}

impl Json {
    pub fn new() -> Self {
        Self::default()
    }

    fn separate(&mut self) {
        if self.after_key {
            self.after_key = false;
        } else if let Some(has_value) = self.open.last_mut() {
            if *has_value {
                self.buf.push(b',');
            }
            *has_value = true;
        }
    }

    pub fn obj(&mut self) -> &mut Self {
        self.separate();
        self.buf.push(b'{');
        self.open.push(false);
        self
    }

    pub fn end_obj(&mut self) -> &mut Self {
        self.open.pop();
        self.buf.push(b'}');
        self
    }

    pub fn arr(&mut self) -> &mut Self {
        self.separate();
        self.buf.push(b'[');
        self.open.push(false);
        self
    }

    pub fn end_arr(&mut self) -> &mut Self {
        self.open.pop();
        self.buf.push(b']');
        self
    }

    pub fn key(&mut self, key: &str) -> &mut Self {
        self.separate();
        escape_into(&mut self.buf, key.as_bytes());
        self.buf.push(b':');
        self.after_key = true;
        self
    }

    pub fn str(&mut self, value: &str) -> &mut Self {
        self.separate();
        escape_into(&mut self.buf, value.as_bytes());
        self
    }

    pub fn int(&mut self, value: i64) -> &mut Self {
        self.separate();
        self.buf.extend_from_slice(value.to_string().as_bytes());
        self
    }

    pub fn float(&mut self, value: f64) -> &mut Self {
        self.separate();
        self.buf.extend_from_slice(printf_f(value).as_bytes());
        self
    }

    pub fn bool(&mut self, value: bool) -> &mut Self {
        self.separate();
        self.buf.extend_from_slice(if value { b"true" } else { b"false" });
        self
    }

    pub fn null(&mut self) -> &mut Self {
        self.separate();
        self.buf.extend_from_slice(b"null");
        self
    }

    pub fn kstr(&mut self, key: &str, value: &str) -> &mut Self {
        self.key(key).str(value)
    }

    pub fn kint(&mut self, key: &str, value: i64) -> &mut Self {
        self.key(key).int(value)
    }

    pub fn kbool(&mut self, key: &str, value: bool) -> &mut Self {
        self.key(key).bool(value)
    }

    pub fn kfloat(&mut self, key: &str, value: f64) -> &mut Self {
        self.key(key).float(value)
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
}

/// A quoted string: `"` `\` and newline, carriage return and tab escaped by name, other bytes below 0x20 as
/// `\u00xx` (lowercase), every other byte (0x7F and non-ASCII included) as it is.
pub fn escape_into(out: &mut Vec<u8>, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    for &byte in bytes {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0..0x20 => out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[usize::from(byte >> 4)],
                HEX[usize::from(byte & 0xF)],
            ]),
            _ => out.push(byte),
        }
    }
    out.push(b'"');
}

pub fn escape(text: &str) -> String {
    let mut out = Vec::new();
    escape_into(&mut out, text.as_bytes());
    String::from_utf8_lossy(&out).into_owned()
}

/// C's `%f` as the MSVC runtime writes it: six decimals, correctly rounded from the exact binary value, no exponent.
/// Infinities and NaNs are written as the UCRT does (spec §5, an ambiguity: none observed).
pub fn printf_f(value: f64) -> String {
    if value.is_nan() {
        const QUIET: u64 = 1 << 51;
        const INDEFINITE: u64 = 0xFFF8_0000_0000_0000;
        let bits = value.to_bits();
        let sign = if value.is_sign_negative() { "-" } else { "" };
        return if bits == INDEFINITE {
            "-nan(ind)".to_owned()
        } else if bits & QUIET == 0 {
            format!("{sign}nan(snan)")
        } else {
            format!("{sign}nan")
        };
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    format!("{value:.6}")
}

/// What a text-mode stream writes for `text`: every `\n` as `\r\n` (spec §5).
pub fn crlf(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + text.len() / 16);
    for &byte in text {
        if byte == b'\n' {
            out.push(b'\r');
        }
        out.push(byte);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_as_the_exporter_does() {
        let mut out = Vec::new();
        escape_into(&mut out, b"a\"b\\c\nd\re\tf\x01\x1f\x7f\xc3\xa9/");
        assert_eq!(out, b"\"a\\\"b\\\\c\\nd\\re\\tf\\u0001\\u001f\x7f\xc3\xa9/\"");
    }

    #[test]
    fn writes_percent_f() {
        assert_eq!(printf_f(1.0), "1.000000");
        assert_eq!(printf_f(-0.5), "-0.500000");
        assert_eq!(printf_f(0.0), "0.000000");
        assert_eq!(printf_f(-0.0), "-0.000000");
        assert_eq!(printf_f(f64::from(0.1f32)), "0.100000");
        assert_eq!(printf_f(f64::from(1.1f32)), "1.100000");
        assert_eq!(printf_f(1e20), "100000000000000000000.000000");
        assert_eq!(printf_f(f64::from(16_777_217.0f32)), "16777216.000000");
        assert_eq!(printf_f(1.0000005), "1.000001");
        assert_eq!(printf_f(2.5e-7), "0.000000");
        assert_eq!(printf_f(f64::INFINITY), "inf");
        assert_eq!(printf_f(f64::NEG_INFINITY), "-inf");
        assert_eq!(printf_f(f64::from_bits(0xFFF8_0000_0000_0000)), "-nan(ind)");
        assert_eq!(printf_f(f64::from_bits(0x7FF8_0000_0000_0000)), "nan");
    }

    #[test]
    fn rounds_exact_ties_to_even() {
        // 0.0078125 and 0.0234375 are exact in binary, so their seventh decimal is a true tie.
        assert_eq!(printf_f(0.007_812_5), "0.007812");
        assert_eq!(printf_f(0.023_437_5), "0.023438");
    }

    #[test]
    fn builds_compact_json() {
        let mut json = Json::new();
        json.obj()
            .kstr("a", "x")
            .key("b")
            .arr()
            .int(1)
            .int(-2)
            .obj()
            .kbool("c", true)
            .end_obj()
            .end_arr();
        json.kfloat("d", 0.25)
            .key("e")
            .arr()
            .end_arr()
            .key("f")
            .null()
            .end_obj();
        assert_eq!(
            json.into_bytes(),
            br#"{"a":"x","b":[1,-2,{"c":true}],"d":0.250000,"e":[],"f":null}"#
        );
    }

    #[test]
    fn writes_crlf() {
        assert_eq!(crlf(b"a\nb\n"), b"a\r\nb\r\n");
        assert_eq!(crlf(b""), b"");
    }
}
