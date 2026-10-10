//! Sun position for shadows.
//!
//! NOAA's solar calculator algorithm (the low-precision Meeus formulas from NOAA's
//! "General Solar Position" spreadsheet), good to roughly 0.01 degrees for years
//! 1901..2099. Unlike NOAA it applies no atmospheric refraction, so near the horizon the
//! sun is up to ~0.6 degrees lower than NOAA's "apparent" elevation. Unverified how
//! SketchUp computes its sun; differences of a fraction of a degree are expected.

use glam::DVec3;

/// Unit vector from the ground towards the sun in model axes (+X east, +Y north, +Z up;
/// SketchUp's default north is +Y, a rotated north must be applied by the caller).
///
/// Local clock time `hour:minute` on `year-month-day` at `utc_offset_hours` (east
/// positive, including any daylight saving). Longitude east positive. The sun is below
/// the horizon when `z <= 0`.
#[allow(clippy::too_many_arguments)]
pub fn sun_direction(
    latitude_deg: f64,
    longitude_deg: f64,
    utc_offset_hours: f64,
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
) -> DVec3 {
    let local_minutes = f64::from(hour) * 60.0 + f64::from(minute);
    let jd = julian_day(year, month, day) + (local_minutes / 60.0 - utc_offset_hours) / 24.0;
    let (decl_deg, eot_min) = solar_coordinates(jd);
    // True solar time in minutes, then the hour angle (negative in the morning).
    let solar_minutes = local_minutes + eot_min + 4.0 * longitude_deg - 60.0 * utc_offset_hours;
    let hour_angle = (solar_minutes / 4.0 - 180.0).to_radians();
    let (lat, decl) = (latitude_deg.to_radians(), decl_deg.to_radians());
    DVec3::new(
        -decl.cos() * hour_angle.sin(),
        lat.cos() * decl.sin() - lat.sin() * decl.cos() * hour_angle.cos(),
        lat.sin() * decl.sin() + lat.cos() * decl.cos() * hour_angle.cos(),
    )
    .normalize()
}

/// Julian day at 0h UT of a Gregorian calendar date (Meeus, Astronomical Algorithms, 7.1).
fn julian_day(year: i32, month: u32, day: u32) -> f64 {
    let (y, m) = if month <= 2 {
        (f64::from(year) - 1.0, f64::from(month) + 12.0)
    } else {
        (f64::from(year), f64::from(month))
    };
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + f64::from(day) + b - 1524.5
}

/// Apparent declination of the sun in degrees and the equation of time in minutes at
/// Julian day `jd` (NOAA spreadsheet formulas; time scale differences ignored).
fn solar_coordinates(jd: f64) -> (f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let mr = m.to_radians();
    let center = mr.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * mr).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * mr).sin() * 0.000289;
    let omega = (125.04 - 1934.136 * t).to_radians();
    let apparent_long = (l0 + center - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let mean_obliquity =
        23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let obliquity = (mean_obliquity + 0.00256 * omega.cos()).to_radians();
    let decl = (obliquity.sin() * apparent_long.sin()).asin();

    let y = (obliquity / 2.0).tan().powi(2);
    let l0r = l0.to_radians();
    let eot = y * (2.0 * l0r).sin() - 2.0 * e * mr.sin()
        + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos()
        - 0.5 * y * y * (4.0 * l0r).sin()
        - 1.25 * e * e * (2.0 * mr).sin();
    (decl.to_degrees(), 4.0 * eot.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elevation_deg(d: DVec3) -> f64 {
        d.z.asin().to_degrees()
    }

    #[test]
    fn julian_day_matches_meeus() {
        // Meeus example 7.a: 1957 October 4.81 = JD 2436116.31.
        assert!((julian_day(1957, 10, 4) + 0.81 - 2_436_116.31).abs() < 1e-9);
        // J2000.0 is 2000 January 1.5.
        assert_eq!(julian_day(2000, 1, 1), 2_451_544.5);
    }

    #[test]
    fn declination_and_equation_of_time_match_meeus() {
        // Meeus, Astronomical Algorithms, 1992 October 13.0 TD (JD 2448908.5):
        // example 25.a gives apparent declination -7.78507 degrees, example 28.b the
        // equation of time +13m 42.6s. TD vs UT (~1 minute) is ignored, which moves
        // these by far less than the tolerances.
        let (decl, eot) = solar_coordinates(julian_day(1992, 10, 13));
        assert!((decl - -7.78507).abs() < 0.01, "declination {decl}");
        assert!((eot - 13.71).abs() < 0.05, "equation of time {eot} min");
    }

    #[test]
    fn solar_noon_is_due_south_at_expected_elevation() {
        // 1992-10-13 at longitude 0, UTC: solar noon at 12:00 - 13.7 min = 11:46.
        let d = sun_direction(40.0, 0.0, 0.0, 1992, 10, 13, 11, 46);
        assert!((d.length() - 1.0).abs() < 1e-12);
        // Declination at that instant (it drifts ~0.4 degrees a day in October).
        let (decl, _) = solar_coordinates(julian_day(1992, 10, 13) + (11.0 * 60.0 + 46.0) / 1440.0);
        assert!((decl - -7.97).abs() < 0.02, "declination {decl}");
        let expected = 90.0 - 40.0 + decl;
        assert!(
            (elevation_deg(d) - expected).abs() < 0.05,
            "elevation {} vs {expected}",
            elevation_deg(d)
        );
        assert!(d.y < 0.0 && d.x.abs() < 0.01, "towards the south, got {d}");
    }

    #[test]
    fn morning_east_afternoon_west_and_night_below_horizon() {
        // Boulder, Colorado (NOAA's home), summer daylight time UTC-6.
        let morning = sun_direction(40.0, -105.0, -6.0, 2024, 6, 21, 8, 0);
        let evening = sun_direction(40.0, -105.0, -6.0, 2024, 6, 21, 18, 0);
        let night = sun_direction(40.0, -105.0, -6.0, 2024, 6, 21, 23, 30);
        assert!(morning.x > 0.0 && morning.z > 0.0, "{morning}");
        assert!(evening.x < 0.0 && evening.z > 0.0, "{evening}");
        assert!(night.z < 0.0, "{night}");
        // Same instant expressed in another time zone gives the same direction.
        let utc = sun_direction(40.0, -105.0, 0.0, 2024, 6, 21, 14, 0);
        assert!(utc.distance(morning) < 1e-12);
    }

    #[test]
    fn southern_summer_noon_sun_is_north() {
        // Sydney around solar noon in late December: the sun is high and to the north.
        let d = sun_direction(-33.87, 151.21, 11.0, 2024, 12, 21, 12, 55);
        assert!(d.y > 0.0, "{d}");
        assert!((elevation_deg(d) - (90.0 - (-23.44_f64 + 33.87).abs())).abs() < 1.0);
    }
}
