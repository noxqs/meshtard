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

    pub const ALL: [Self; 5] = [
        Self::Inch,
        Self::Foot,
        Self::Millimeter,
        Self::Centimeter,
        Self::Meter,
    ];

    /// Model Info > Units name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Inch => "Inches",
            Self::Foot => "Feet",
            Self::Millimeter => "Millimeters",
            Self::Centimeter => "Centimeters",
            Self::Meter => "Meters",
        }
    }

    /// Formats inches in this unit, e.g. `1200mm`, with the display options set by
    /// [`set_display`] (two decimals at most without them).
    pub fn format(self, inches: f64) -> String {
        if let Some(opts) = display() {
            return format_length(inches, self, &opts);
        }
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
    // Fractions: `1/2`, `6 1/2`.
    if let Some((whole, frac)) = s.split_once('/').map(|(a, b)| (a.trim(), b.trim())) {
        let (whole, num) = match whole.rsplit_once(' ') {
            Some((w, n)) => (w.trim().parse::<f64>().ok()?, n.parse::<f64>().ok()?),
            None => (0.0, whole.parse::<f64>().ok()?),
        };
        let den: f64 = frac.parse().ok()?;
        if den == 0.0 {
            return None;
        }
        let f = num / den;
        return Some(if whole < 0.0 || s.starts_with('-') {
            whole - f
        } else {
            whole + f
        });
    }
    s.parse().ok()
}

/// Model Info > Units length formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LengthFormat {
    /// Feet and fractional inches: `5' 6 1/2"`.
    Architectural,
    /// Decimal numbers in any unit: `1200mm`.
    #[default]
    Decimal,
    /// Decimal feet: `5.50'`.
    Engineering,
    /// Fractional inches: `66 1/2"`.
    Fractional,
}

impl LengthFormat {
    pub const ALL: [Self; 4] = [
        Self::Architectural,
        Self::Decimal,
        Self::Engineering,
        Self::Fractional,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Architectural => "Architectural",
            Self::Decimal => "Decimal",
            Self::Engineering => "Engineering",
            Self::Fractional => "Fractional",
        }
    }

    /// The unit every format but Decimal is tied to.
    pub fn fixed_unit(self) -> Option<LengthUnit> {
        match self {
            Self::Architectural | Self::Fractional => Some(LengthUnit::Inch),
            Self::Engineering => Some(LengthUnit::Foot),
            Self::Decimal => None,
        }
    }

    /// Whether precision is a fraction (`1/2^precision` inch) rather than decimals.
    pub fn is_fractional(self) -> bool {
        matches!(self, Self::Architectural | Self::Fractional)
    }
}

/// Model Info > Units: how lengths and angles are shown and snapped. The unit itself
/// is kept apart (the document's `units`). Defaults are drafts (SketchUp's
/// templates differ; unverified).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitsOptions {
    pub format: LengthFormat,
    /// Decimal places, or for fractional formats the denominator's power of two
    /// (4 = 1/16").
    pub precision: u8,
    pub length_snapping: bool,
    /// Inches.
    pub snap_length: f64,
    /// Decimal places of angles.
    pub angle_precision: u8,
    pub angle_snapping: bool,
    /// Degrees.
    pub snap_angle: f64,
}

impl Default for UnitsOptions {
    fn default() -> Self {
        Self {
            format: LengthFormat::Decimal,
            precision: 2,
            length_snapping: true,
            snap_length: 0.25,
            angle_precision: 1,
            angle_snapping: true,
            snap_angle: 15.0,
        }
    }
}

/// Display options in effect for [`LengthUnit::format`]; `None` keeps the plain
/// two-decimal format.
static DISPLAY: std::sync::RwLock<Option<UnitsOptions>> = std::sync::RwLock::new(None);

/// Sets the display options of the open model (one model is shown at a time).
pub fn set_display(opts: Option<UnitsOptions>) {
    if let Ok(mut d) = DISPLAY.write() {
        *d = opts;
    }
}

fn display() -> Option<UnitsOptions> {
    DISPLAY.read().ok().and_then(|d| *d)
}

/// A rounded value is marked with `~ `, as SketchUp does.
fn approx(exact: f64, shown: f64) -> &'static str {
    if (exact - shown).abs() > 1e-9 * exact.abs().max(1.0) {
        "~ "
    } else {
        ""
    }
}

/// `v` (non-negative) rounded to `1/den`: whole part, numerator, rounded value.
fn fraction(v: f64, den: u32) -> (u64, u32, f64) {
    let ticks = (v * f64::from(den)).round() as u64;
    let rounded = ticks as f64 / f64::from(den);
    (
        ticks / u64::from(den),
        (ticks % u64::from(den)) as u32,
        rounded,
    )
}

fn fraction_text(whole: u64, mut num: u32, mut den: u32) -> String {
    while num != 0 && num.is_multiple_of(2) {
        num /= 2;
        den /= 2;
    }
    match (whole, num) {
        (w, 0) => w.to_string(),
        (0, n) => format!("{n}/{den}"),
        (w, n) => format!("{w} {n}/{den}"),
    }
}

/// Formats inches as Model Info > Units says. `unit` is used by Decimal.
pub fn format_length(inches: f64, unit: LengthUnit, opts: &UnitsOptions) -> String {
    let sign = if inches < 0.0 { "-" } else { "" };
    let abs = inches.abs();
    match opts.format {
        LengthFormat::Decimal | LengthFormat::Engineering => {
            let unit = opts.format.fixed_unit().unwrap_or(unit);
            let v = abs / unit.inches();
            let p = usize::from(opts.precision.min(6));
            let text = format!("{v:.p$}");
            let shown: f64 = text.parse().unwrap_or(v);
            let sign = if shown == 0.0 { "" } else { sign };
            format!("{}{sign}{text}{}", approx(v, shown), unit.suffix())
        }
        LengthFormat::Fractional => {
            let den = 1u32 << opts.precision.min(6);
            let (whole, num, rounded) = fraction(abs, den);
            let sign = if rounded == 0.0 { "" } else { sign };
            format!(
                "{}{sign}{}\"",
                approx(abs, rounded),
                fraction_text(whole, num, den)
            )
        }
        LengthFormat::Architectural => {
            let den = 1u32 << opts.precision.min(6);
            let (whole, num, rounded) = fraction(abs, den);
            let sign = if rounded == 0.0 { "" } else { sign };
            let (feet, inch) = (whole / 12, whole % 12);
            let inches = fraction_text(inch, num, den);
            // Under a foot only inches are shown (unverified).
            let text = if feet > 0 {
                format!("{feet}' {inches}\"")
            } else {
                format!("{inches}\"")
            };
            format!("{}{sign}{text}", approx(abs, rounded))
        }
    }
}

/// Formats an angle in degrees with the Model Info angle precision.
pub fn format_angle(deg: f64, opts: &UnitsOptions) -> String {
    let p = usize::from(opts.angle_precision.min(6));
    let text = format!("{deg:.p$}");
    let shown: f64 = text.parse().unwrap_or(deg);
    format!("{}{text}°", approx(deg, shown))
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
            !(c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | '/') || c.is_whitespace())
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

    #[test]
    fn formats_follow_model_info() {
        let mut o = UnitsOptions {
            format: LengthFormat::Decimal,
            precision: 0,
            ..UnitsOptions::default()
        };
        let mm = LengthUnit::Millimeter;
        assert_eq!(format_length(1200.0 / 25.4, mm, &o), "1200mm");
        assert_eq!(format_length(1.0, mm, &o), "~ 25mm");
        o.precision = 2;
        assert_eq!(format_length(12.5, LengthUnit::Inch, &o), "12.50\"");
        o.format = LengthFormat::Engineering;
        assert_eq!(format_length(66.0, mm, &o), "5.50'");
        o.format = LengthFormat::Fractional;
        o.precision = 4;
        assert_eq!(format_length(66.5, mm, &o), "66 1/2\"");
        assert_eq!(format_length(0.03, mm, &o), "~ 0\"");
        o.format = LengthFormat::Architectural;
        assert_eq!(format_length(66.5, mm, &o), "5' 6 1/2\"");
        assert_eq!(format_length(60.0, mm, &o), "5' 0\"");
        assert_eq!(format_length(-6.25, mm, &o), "-6 1/4\"");
        assert_eq!(format_length(66.53, mm, &o), "~ 5' 6 1/2\"");
        assert_eq!(format_angle(12.345, &o), "~ 12.3°");
    }

    #[test]
    fn fractions_parse() {
        let inch = LengthUnit::Inch;
        assert!(close(parse_length("6 1/2", inch), 6.5));
        assert!(close(parse_length("1/4\"", inch), 0.25));
        assert!(close(parse_length("5' 6 1/2\"", inch), 66.5));
        assert_eq!(parse_length("1/0", inch), None);
    }
}
