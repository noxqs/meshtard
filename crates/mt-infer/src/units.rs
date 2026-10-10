//! Lengths typed into the Measurements box.
//!
//! Accepts a number with an optional unit (`1200mm`, `1.2m`, `30cm`, `12"`, `12in`,
//! `5'`, `5ft`, `4'6"` feet and inches). A number without a unit is in the model's
//! display unit. Decimal comma is accepted (`1,5m`). Results are inches.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    Inch,
    Foot,
    Millimeter,
    Centimeter,
    Meter,
}

impl LengthUnit {
    pub fn inches(self) -> f64 {
        match self {
            Self::Inch => 1.0,
            Self::Foot => 12.0,
            Self::Millimeter => 1.0 / 25.4,
            Self::Centimeter => 1.0 / 2.54,
            Self::Meter => 1000.0 / 25.4,
        }
    }

    /// From SketchUp's unit names (`meta/meta.dat`) and common spellings.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "in" | "inch" | "inches" | "\"" => Self::Inch,
            "ft" | "foot" | "feet" | "'" => Self::Foot,
            "mm" | "millimeter" | "millimeters" | "millimetre" => Self::Millimeter,
            "cm" | "centimeter" | "centimeters" | "centimetre" => Self::Centimeter,
            "m" | "meter" | "meters" | "metre" => Self::Meter,
            _ => return None,
        })
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::Inch => "\"",
            Self::Foot => "'",
            Self::Millimeter => "mm",
            Self::Centimeter => "cm",
            Self::Meter => "m",
        }
    }

    /// Formats inches in this unit, e.g. `1200mm`.
    pub fn format(self, inches: f64) -> String {
        let v = inches / self.inches();
        let text = format!("{v:.2}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        format!("{text}{}", self.suffix())
    }
}

fn number(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    if s.is_empty() {
        return None;
    }
    s.parse().ok()
}

/// Parses a typed length into inches; `None` when unparsable.
pub fn parse_length(text: &str, default_unit: LengthUnit) -> Option<f64> {
    let t = text.trim();
    // Feet and inches: 4'6" or 4' 6".
    if let Some((feet, rest)) = t.split_once('\'') {
        let rest = rest.trim();
        let f = number(feet)?;
        let inches = if rest.is_empty() {
            0.0
        } else {
            number(rest.strip_suffix('"').unwrap_or(rest))?
        };
        let total = f.abs() * 12.0 + inches;
        return Some(if feet.trim().starts_with('-') {
            -total
        } else {
            total
        });
    }
    let split = t
        .find(|c: char| {
            !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+') || c.is_whitespace())
        })
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let v = number(num)?;
    let unit = if unit.trim().is_empty() {
        default_unit
    } else {
        LengthUnit::from_name(unit)?
    };
    Some(v * unit.inches())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn units() {
        let mm = LengthUnit::Millimeter;
        assert!(close(
            parse_length("1200mm", LengthUnit::Inch),
            1200.0 / 25.4
        ));
        assert!(close(parse_length("1.2m", mm), 1200.0 / 25.4));
        assert!(close(parse_length("1,2 m", mm), 1200.0 / 25.4));
        assert!(close(parse_length("30cm", mm), 300.0 / 25.4));
        assert!(close(parse_length("12\"", mm), 12.0));
        assert!(close(parse_length("5'", mm), 60.0));
        assert!(close(parse_length("4'6\"", mm), 54.0));
        assert!(close(parse_length("4' 6", mm), 54.0));
        assert!(close(parse_length("100", mm), 100.0 / 25.4));
        assert!(close(parse_length("-10\"", mm), -10.0));
        assert_eq!(parse_length("abc", mm), None);
        assert_eq!(parse_length("", mm), None);
        assert_eq!(parse_length("10parsecs", mm), None);
    }

    #[test]
    fn format_round_trips() {
        let mm = LengthUnit::Millimeter;
        assert_eq!(mm.format(1200.0 / 25.4), "1200mm");
        assert_eq!(LengthUnit::Inch.format(12.5), "12.5\"");
        assert!(close(parse_length(&mm.format(10.0), mm), 10.0));
    }
}
