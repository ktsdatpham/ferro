use serde::{Deserialize, Serialize};

/// A length parsed from user or agent input, e.g. "3000", "3m", "300cm", "10'".
/// Stored as millimetres, which is also the kernel's internal unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Length(pub f64);

impl Length {
    pub fn mm(self) -> f64 {
        self.0
    }

    /// Parse a length string. A bare number means millimetres.
    pub fn parse(s: &str) -> Option<Length> {
        let s = s.trim().to_lowercase();
        let (num, factor) = if let Some(n) = s.strip_suffix("mm") {
            (n, 1.0)
        } else if let Some(n) = s.strip_suffix("cm") {
            (n, 10.0)
        } else if let Some(n) = s.strip_suffix('m') {
            (n, 1000.0)
        } else if let Some(n) = s.strip_suffix('"').or_else(|| s.strip_suffix("in")) {
            (n, 25.4)
        } else if let Some(n) = s.strip_suffix('\'').or_else(|| s.strip_suffix("ft")) {
            (n, 304.8)
        } else {
            (s.as_str(), 1.0)
        };
        num.trim().parse::<f64>().ok().map(|v| Length(v * factor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_units() {
        assert_eq!(Length::parse("3000").unwrap().mm(), 3000.0);
        assert_eq!(Length::parse("3m").unwrap().mm(), 3000.0);
        assert_eq!(Length::parse("300 cm").unwrap().mm(), 3000.0);
        assert_eq!(Length::parse("10'").unwrap().mm(), 3048.0);
        assert!(Length::parse("abc").is_none());
    }
}
