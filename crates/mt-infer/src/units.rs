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
    /// Area and volume display units and their decimal places.
    pub area_unit: AreaUnit,
    pub area_precision: u8,
    pub volume_unit: VolumeUnit,
    pub volume_precision: u8,
    /// "Force display of 0\"": Architectural whole feet show `5' 0"` instead of `5'`
    /// (the file's `ForceInchDisplay`).
    pub force_inch_display: bool,
    /// "Display units format" unchecked: Decimal lengths without the unit symbol (the
    /// file's `SuppressUnitsDisplay`).
    pub suppress_units: bool,
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
            area_unit: AreaUnit::SquareInches,
            area_precision: 2,
            volume_unit: VolumeUnit::CubicInches,
            volume_precision: 2,
            // Kept on so Architectural shows `5' 0"` as before (SketchUp's template
            // default is unverified).
            force_inch_display: true,
            suppress_units: false,
        }
    }
}

/// Model Info > Units > Area. Codes are the file's `AreaUnit` values (Ruby
/// `Length::SquareInches` ... `Length::SquareYards`; unverified).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AreaUnit {
    #[default]
    SquareInches,
    SquareFeet,
    SquareMillimeters,
    SquareCentimeters,
    SquareMeters,
    SquareYards,
}

impl AreaUnit {
    pub const ALL: [Self; 6] = [
        Self::SquareInches,
        Self::SquareFeet,
        Self::SquareMillimeters,
        Self::SquareCentimeters,
        Self::SquareMeters,
        Self::SquareYards,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SquareInches => "Square Inches",
            Self::SquareFeet => "Square Feet",
            Self::SquareMillimeters => "Square Millimeters",
            Self::SquareCentimeters => "Square Centimeters",
            Self::SquareMeters => "Square Meters",
            Self::SquareYards => "Square Yards",
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::SquareInches => "in²",
            Self::SquareFeet => "ft²",
            Self::SquareMillimeters => "mm²",
            Self::SquareCentimeters => "cm²",
            Self::SquareMeters => "m²",
            Self::SquareYards => "yd²",
        }
    }

    /// Square inches in one unit.
    pub fn square_inches(self) -> f64 {
        let side = match self {
            Self::SquareInches => 1.0,
            Self::SquareFeet => 12.0,
            Self::SquareMillimeters => 1.0 / 25.4,
            Self::SquareCentimeters => 1.0 / 2.54,
            Self::SquareMeters => 1000.0 / 25.4,
            Self::SquareYards => 36.0,
        };
        side * side
    }

    pub fn code(self) -> i32 {
        Self::ALL.iter().position(|u| *u == self).unwrap_or(0) as i32
    }

    pub fn from_code(code: i32) -> Option<Self> {
        usize::try_from(code)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// The square of a length unit (files without an area unit).
    pub fn of_length(unit: LengthUnit) -> Self {
        match unit {
            LengthUnit::Inch => Self::SquareInches,
            LengthUnit::Foot => Self::SquareFeet,
            LengthUnit::Millimeter => Self::SquareMillimeters,
            LengthUnit::Centimeter => Self::SquareCentimeters,
            LengthUnit::Meter => Self::SquareMeters,
        }
    }
}

/// Model Info > Units > Volume. Codes are the file's `VolumeUnit` values (Ruby
/// `Length::CubicInches` ... `Length::USGallons`; unverified).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VolumeUnit {
    #[default]
    CubicInches,
    CubicFeet,
    CubicMillimeters,
    CubicCentimeters,
    CubicMeters,
    CubicYards,
    Liters,
    UsGallons,
}

impl VolumeUnit {
    pub const ALL: [Self; 8] = [
        Self::CubicInches,
        Self::CubicFeet,
        Self::CubicMillimeters,
        Self::CubicCentimeters,
        Self::CubicMeters,
        Self::CubicYards,
        Self::Liters,
        Self::UsGallons,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::CubicInches => "Cubic Inches",
            Self::CubicFeet => "Cubic Feet",
            Self::CubicMillimeters => "Cubic Millimeters",
            Self::CubicCentimeters => "Cubic Centimeters",
            Self::CubicMeters => "Cubic Meters",
            Self::CubicYards => "Cubic Yards",
            Self::Liters => "Liters",
            Self::UsGallons => "US Gallons",
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::CubicInches => "in³",
            Self::CubicFeet => "ft³",
            Self::CubicMillimeters => "mm³",
            Self::CubicCentimeters => "cm³",
            Self::CubicMeters => "m³",
            Self::CubicYards => "yd³",
            Self::Liters => "L",
            Self::UsGallons => "gal",
        }
    }

    /// Cubic inches in one unit.
    pub fn cubic_inches(self) -> f64 {
        let cube = |side: f64| side * side * side;
        match self {
            Self::CubicInches => 1.0,
            Self::CubicFeet => cube(12.0),
            Self::CubicMillimeters => cube(1.0 / 25.4),
            Self::CubicCentimeters => cube(1.0 / 2.54),
            Self::CubicMeters => cube(1000.0 / 25.4),
            Self::CubicYards => cube(36.0),
            Self::Liters => cube(10.0 / 2.54),
            Self::UsGallons => 231.0,
        }
    }

    pub fn code(self) -> i32 {
        Self::ALL.iter().position(|u| *u == self).unwrap_or(0) as i32
    }

    pub fn from_code(code: i32) -> Option<Self> {
        usize::try_from(code)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// The cube of a length unit (files without a volume unit).
    pub fn of_length(unit: LengthUnit) -> Self {
        match unit {
            LengthUnit::Inch => Self::CubicInches,
            LengthUnit::Foot => Self::CubicFeet,
            LengthUnit::Millimeter => Self::CubicMillimeters,
            LengthUnit::Centimeter => Self::CubicCentimeters,
            LengthUnit::Meter => Self::CubicMeters,
        }
    }
}

/// `value` with `precision` decimals and the `~ ` mark when rounded.
fn decimal_text(value: f64, precision: u8) -> String {
    let p = usize::from(precision.min(6));
    let text = format!("{value:.p$}");
    let shown: f64 = text.parse().unwrap_or(value);
    format!("{}{text}", approx(value, shown))
}

/// An area in square inches in Model Info's area unit and precision, e.g.
/// `12.50 ft²` (SketchUp's exact spacing unverified).
pub fn format_area(square_inches: f64, opts: &UnitsOptions) -> String {
    let v = square_inches / opts.area_unit.square_inches();
    format!(
        "{} {}",
        decimal_text(v, opts.area_precision),
        opts.area_unit.suffix()
    )
}

/// A volume in cubic inches in Model Info's volume unit and precision.
pub fn format_volume(cubic_inches: f64, opts: &UnitsOptions) -> String {
    let v = cubic_inches / opts.volume_unit.cubic_inches();
    format!(
        "{} {}",
        decimal_text(v, opts.volume_precision),
        opts.volume_unit.suffix()
    )
}

/// Length snapping: `length` (inches) rounded to the snap length when enabled.
pub fn snap_length(length: f64, opts: &UnitsOptions) -> f64 {
    if !opts.length_snapping || opts.snap_length <= 0.0 || !length.is_finite() {
        return length;
    }
    (length / opts.snap_length).round() * opts.snap_length
}

/// Angle snapping: `angle` (radians) moves to the nearest multiple of the snap angle
/// when it is within a quarter of the snap angle of it (how close SketchUp must be is
/// unverified).
pub fn snap_angle(angle: f64, opts: &UnitsOptions) -> f64 {
    if !opts.angle_snapping || opts.snap_angle <= 0.0 || !angle.is_finite() {
        return angle;
    }
    let step = opts.snap_angle.to_radians();
    let k = (angle / step).round();
    if (angle - k * step).abs() <= step * 0.25 {
        k * step
    } else {
        angle
    }
}

/// [`snap_angle`] with the open model's options ([`set_display`]); unchanged without.
pub fn snap_angle_display(angle: f64) -> f64 {
    display().map_or(angle, |o| snap_angle(angle, &o))
}

/// An angle in degrees for the Measurements box: the number only, with Model Info's
/// angle precision (one decimal without a model).
pub fn angle_text(deg: f64) -> String {
    let p = usize::from(display().map_or(1, |o| o.angle_precision).min(6));
    format!("{deg:.p$}")
}

/// The open model's display options, if any.
pub fn display_options() -> Option<UnitsOptions> {
    display()
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
            let suffix = if opts.suppress_units && opts.format == LengthFormat::Decimal {
                ""
            } else {
                unit.suffix()
            };
            format!("{}{sign}{text}{suffix}", approx(v, shown))
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
            // Under a foot only inches are shown (unverified). Whole feet show their
            // zero inches only with "Force display of 0\"".
            let text = if feet > 0 && inch == 0 && num == 0 && !opts.force_inch_display {
                format!("{feet}'")
            } else if feet > 0 {
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
        o.force_inch_display = false;
        assert_eq!(format_length(60.0, mm, &o), "5'");
        assert_eq!(format_length(66.5, mm, &o), "5' 6 1/2\"");
        o.format = LengthFormat::Decimal;
        o.suppress_units = true;
        o.precision = 1;
        assert_eq!(format_length(12.5, LengthUnit::Inch, &o), "12.5");
    }

    #[test]
    fn areas_and_volumes_in_their_units() {
        let mut o = UnitsOptions {
            area_unit: AreaUnit::SquareFeet,
            area_precision: 2,
            volume_unit: VolumeUnit::Liters,
            volume_precision: 1,
            ..UnitsOptions::default()
        };
        assert_eq!(format_area(144.0 * 2.5, &o), "2.50 ft²");
        // One liter is 61.0237 cubic inches.
        assert_eq!(format_volume(61.023_744, &o), "~ 1.0 L");
        o.area_unit = AreaUnit::SquareMeters;
        o.area_precision = 0;
        assert_eq!(format_area(1600.0, &o), "~ 1 m²");
        for u in AreaUnit::ALL {
            assert_eq!(AreaUnit::from_code(u.code()), Some(u));
        }
        for u in VolumeUnit::ALL {
            assert_eq!(VolumeUnit::from_code(u.code()), Some(u));
        }
        assert_eq!(VolumeUnit::from_code(8), None);
        assert!((VolumeUnit::UsGallons.cubic_inches() - 231.0).abs() < 1e-12);
    }

    #[test]
    fn snapping_rounds_lengths_and_angles() {
        let mut o = UnitsOptions {
            length_snapping: true,
            snap_length: 0.25,
            angle_snapping: true,
            snap_angle: 15.0,
            ..UnitsOptions::default()
        };
        assert!((snap_length(10.13, &o) - 10.25).abs() < 1e-12);
        assert!((snap_length(10.1, &o) - 10.0).abs() < 1e-12);
        let deg = |d: f64| d.to_radians();
        assert!((snap_angle(deg(44.0), &o) - deg(45.0)).abs() < 1e-12);
        assert!((snap_angle(deg(-31.0), &o) - deg(-30.0)).abs() < 1e-12);
        // Between ticks the angle is free.
        assert!((snap_angle(deg(37.0), &o) - deg(37.0)).abs() < 1e-12);
        o.length_snapping = false;
        o.angle_snapping = false;
        assert_eq!(snap_length(10.13, &o), 10.13);
        assert_eq!(snap_angle(deg(44.0), &o), deg(44.0));
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
