//! Small text helpers for the services client: sanitising what the server
//! sends before it is shown, base64url, RFC 3339 UTC times, hex and URL
//! query encoding. No dependencies.

/// Clean text from the server before it is shown: whitespace (newlines,
/// tabs) becomes a space; control characters, bidirectional overrides and
/// zero-width characters are dropped; runs of spaces collapse; the result
/// is trimmed and cut to `max_chars` characters (with `...` when cut).
pub fn sanitize(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 4));
    let mut space = false;
    let mut count = 0usize;
    for c in s.chars() {
        let c = if c.is_whitespace() { ' ' } else { c };
        let hidden = c.is_control()
            || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
        if hidden {
            continue;
        }
        if c == ' ' {
            if space || out.is_empty() {
                continue;
            }
            space = true;
        } else {
            space = false;
        }
        if count == max_chars {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push_str("...");
            return out;
        }
        out.push(c);
        count += 1;
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// True for an id the server may give for a plugin, publisher or order:
/// 1 to 128 characters from `[A-Za-z0-9._-]`, so it is safe in a URL path.
pub fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && s != "."
        && s != ".."
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding (RFC 4648 §5).
pub fn b64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Decode base64url (padding `=` accepted and ignored). `None` on any
/// other character or an impossible length.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_end_matches('=');
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Lower-case hex of bytes.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Percent-encode a URL query value (unreserved characters kept).
pub fn query_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

/// Parse an RFC 3339 time (`2026-11-24T06:00:00Z`, optional fraction,
/// `Z` or `±hh:mm`) into Unix seconds. `None` when malformed.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<i64> {
        let t = s.get(from..from + len)?;
        if !t.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        t.parse().ok()
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, se) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
    }
    let offset = match b.get(i) {
        Some(b'Z' | b'z') if i + 1 == b.len() => 0,
        Some(sign @ (b'+' | b'-')) if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1, 2)?, num(i + 4, 2)?);
            let o = oh * 3600 + om * 60;
            if *sign == b'+' {
                o
            } else {
                -o
            }
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Format Unix seconds as RFC 3339 UTC (`2026-11-24T06:00:00Z`).
pub fn format_rfc3339(t: i64) -> String {
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// Now in Unix seconds (0 if the clock is before 1970).
pub fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_drops_controls_and_bidi_and_cuts() {
        assert_eq!(sanitize("  a\n\tb\u{202E}c\u{0007}  ", 50), "a bc");
        assert_eq!(sanitize("abcdef", 3), "abc...");
        assert_eq!(sanitize("", 3), "");
        assert_eq!(sanitize("Bản vẽ đẹp", 100), "Bản vẽ đẹp");
    }

    #[test]
    fn safe_ids() {
        assert!(is_safe_id("acme.stairs-2_x"));
        for bad in ["", ".", "..", "a/b", "a b", "a%2F", &"x".repeat(129)] {
            assert!(!is_safe_id(bad), "{bad}");
        }
    }

    #[test]
    fn base64url_round_trips() {
        for n in 0..40u8 {
            let v: Vec<u8> = (0..n).map(|i| i.wrapping_mul(37)).collect();
            let e = b64url_encode(&v);
            assert!(!e.contains('=') && !e.contains('+') && !e.contains('/'));
            assert_eq!(b64url_decode(&e).as_deref(), Some(&v[..]));
        }
        assert_eq!(b64url_encode(b"hi?"), "aGk_");
        assert_eq!(b64url_decode("aGk=").as_deref(), Some(&b"hi"[..]));
        assert!(b64url_decode("a").is_none());
        assert!(b64url_decode("ab+c").is_none());
    }

    #[test]
    fn rfc3339_parses_and_formats() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        let t = parse_rfc3339("2026-11-24T06:00:00Z").unwrap();
        assert_eq!(format_rfc3339(t), "2026-11-24T06:00:00Z");
        assert_eq!(parse_rfc3339("2026-11-24T13:00:00.123+07:00"), Some(t));
        assert_eq!(
            parse_rfc3339("2024-02-29T00:00:00Z")
                .map(format_rfc3339)
                .as_deref(),
            Some("2024-02-29T00:00:00Z")
        );
        for bad in [
            "",
            "2026-11-24",
            "2026-13-01T00:00:00Z",
            "2026-11-24T06:00:00",
            "2026-11-24T06:00:00Zjunk",
            "2026-11-24T06:00:00.Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }
}
