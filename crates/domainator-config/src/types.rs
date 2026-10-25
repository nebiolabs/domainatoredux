//! Option value types, and what the manifest says about each.
//!
//! Upstream checks numeric options with restricted types (jsonargparse's `PositiveInt`,
//! `ClosedUnitInterval`, ...), and `domainator describe` reports their bounds. The
//! newtypes here are the same checks as clap value types, so a bound is enforced
//! wherever a value arrives (command line, `--config` file, GUI) and is visible to
//! [`type_info`], which reads it off the argument's value parser. Every float type
//! rejects `nan` and `inf`, as upstream's do.

use std::any::TypeId;
use std::fmt;
use std::str::FromStr;

/// What `describe` reports for an option's value: its JSON type, and its bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TypeInfo {
    pub json_type: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub exclusive_min: Option<f64>,
}

const fn plain(json_type: &'static str) -> TypeInfo {
    TypeInfo {
        json_type,
        min: None,
        max: None,
        exclusive_min: None,
    }
}

/// The manifest type of a value parsed into `id`. Unknown types are strings, which
/// is what an option without a declared type is upstream too.
pub fn type_info(id: &clap::builder::ValueParser) -> TypeInfo {
    let t = id.type_id();
    let is = |other: TypeId| t == other;
    if is(TypeId::of::<PositiveInt>()) {
        TypeInfo {
            min: Some(1.0),
            ..plain("integer")
        }
    } else if is(TypeId::of::<PositiveFloat>()) {
        TypeInfo {
            exclusive_min: Some(0.0),
            ..plain("number")
        }
    } else if is(TypeId::of::<NonNegFloat>()) {
        TypeInfo {
            min: Some(0.0),
            ..plain("number")
        }
    } else if is(TypeId::of::<UnitInterval>()) {
        TypeInfo {
            min: Some(0.0),
            max: Some(1.0),
            ..plain("number")
        }
    } else if is(TypeId::of::<u8>())
        || is(TypeId::of::<u16>())
        || is(TypeId::of::<u32>())
        || is(TypeId::of::<u64>())
        || is(TypeId::of::<usize>())
    {
        TypeInfo {
            min: Some(0.0),
            ..plain("integer")
        }
    } else if is(TypeId::of::<i32>()) || is(TypeId::of::<i64>()) || is(TypeId::of::<isize>()) {
        plain("integer")
    } else if is(TypeId::of::<f64>()) || is(TypeId::of::<f32>()) || is(TypeId::of::<Finite>()) {
        plain("number")
    } else if is(TypeId::of::<bool>()) {
        plain("boolean")
    } else {
        plain("string")
    }
}

/// A finite `f64` (no `nan`, no `inf`), for float options without bounds.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Finite(pub f64);

/// An integer of at least 1 (upstream `PositiveInt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PositiveInt(pub u64);

/// A finite float greater than 0 (upstream `PositiveFloat`).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct PositiveFloat(pub f64);

/// A finite float of at least 0 (upstream `NonNegativeFloat`).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct NonNegFloat(pub f64);

/// A float in [0, 1] (upstream `ClosedUnitInterval`).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct UnitInterval(pub f64);

fn parse_finite(s: &str) -> Result<f64, String> {
    let v: f64 = s
        .trim()
        .parse()
        .map_err(|_| format!("expected a number, got {s:?}"))?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("expected a finite number, got {s:?}"))
    }
}

impl FromStr for Finite {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        parse_finite(s).map(Finite)
    }
}

impl FromStr for PositiveInt {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().parse::<i64>() {
            Ok(v) if v >= 1 => Ok(PositiveInt(v.unsigned_abs())),
            Ok(_) => Err(format!("expected an integer of at least 1, got {s:?}")),
            Err(_) => Err(format!("expected an integer, got {s:?}")),
        }
    }
}

impl FromStr for PositiveFloat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match parse_finite(s)? {
            v if v > 0.0 => Ok(PositiveFloat(v)),
            _ => Err(format!("expected a number greater than 0, got {s:?}")),
        }
    }
}

impl FromStr for NonNegFloat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match parse_finite(s)? {
            v if v >= 0.0 => Ok(NonNegFloat(v)),
            _ => Err(format!("expected a number of at least 0, got {s:?}")),
        }
    }
}

impl FromStr for UnitInterval {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match parse_finite(s)? {
            v if (0.0..=1.0).contains(&v) => Ok(UnitInterval(v)),
            _ => Err(format!("expected a number from 0 to 1, got {s:?}")),
        }
    }
}

macro_rules! display_inner {
    ($($t:ty),*) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}
display_inner!(
    Finite,
    PositiveInt,
    PositiveFloat,
    NonNegFloat,
    UnitInterval
);

#[cfg(test)]
mod tests {
    use super::*;
    use clap::builder::ValueParser;

    #[test]
    fn every_float_type_rejects_nan_and_inf() {
        for s in ["nan", "NaN", "inf", "-inf", "infinity"] {
            assert!(s.parse::<Finite>().is_err(), "Finite {s}");
            assert!(s.parse::<PositiveFloat>().is_err(), "PositiveFloat {s}");
            assert!(s.parse::<NonNegFloat>().is_err(), "NonNegFloat {s}");
            assert!(s.parse::<UnitInterval>().is_err(), "UnitInterval {s}");
        }
    }

    #[test]
    fn bounds_are_enforced_at_their_edges() {
        assert!("0".parse::<PositiveInt>().is_err());
        assert_eq!("1".parse::<PositiveInt>(), Ok(PositiveInt(1)));
        assert!("0".parse::<PositiveFloat>().is_err());
        assert!("0".parse::<NonNegFloat>().is_ok());
        assert!("-0.1".parse::<NonNegFloat>().is_err());
        assert!("1".parse::<UnitInterval>().is_ok());
        assert!("1.01".parse::<UnitInterval>().is_err());
    }

    #[test]
    fn the_manifest_reads_bounds_off_the_value_parser() {
        let info = type_info(&ValueParser::new(|s: &str| s.parse::<UnitInterval>()));
        assert_eq!(info.json_type, "number");
        assert_eq!((info.min, info.max), (Some(0.0), Some(1.0)));
        assert_eq!(
            type_info(&ValueParser::new(|s: &str| s.parse::<PositiveInt>())).min,
            Some(1.0)
        );
        assert_eq!(type_info(&ValueParser::string()).json_type, "string");
    }
}
