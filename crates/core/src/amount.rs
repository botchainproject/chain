//! Token amounts: u64 base units that always cross JSON as a decimal string.

use std::fmt;
use std::str::FromStr;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};

/// An amount of botchain base units.
///
/// In JSON an amount is always a decimal string, for example `"12345"`, so
/// that clients in languages with 53 bit numbers cannot lose precision.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Amount(u64);

/// Error returned when a string is not a plain decimal amount.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AmountError {
    /// The string was empty.
    #[error("amount is empty")]
    Empty,
    /// The string contained something other than ascii digits.
    #[error("amount must be decimal digits, got {0:?}")]
    NotDecimal(String),
    /// The value does not fit in u64.
    #[error("amount {0:?} does not fit in u64")]
    Overflow(String),
}

impl Amount {
    /// Zero.
    pub const ZERO: Amount = Amount(0);

    /// The largest representable amount.
    pub const MAX: Amount = Amount(u64::MAX);

    /// Wraps a raw base unit count.
    pub const fn from_u64(value: u64) -> Self {
        Amount(value)
    }

    /// The raw base unit count.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// True when the amount is zero.
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Adds two amounts, `None` on overflow.
    pub fn checked_add(self, other: Amount) -> Option<Amount> {
        self.0.checked_add(other.0).map(Amount)
    }

    /// Subtracts an amount, `None` when the result would be negative.
    pub fn checked_sub(self, other: Amount) -> Option<Amount> {
        self.0.checked_sub(other.0).map(Amount)
    }

    /// Parses a plain decimal string such as `"1000"`.
    ///
    /// No sign, no underscores, no whitespace and no leading `+`; leading
    /// zeros are accepted because `"007"` is unambiguous.
    pub fn parse_decimal(s: &str) -> Result<Self, AmountError> {
        if s.is_empty() {
            return Err(AmountError::Empty);
        }
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(AmountError::NotDecimal(s.to_string()));
        }
        s.parse::<u64>()
            .map(Amount)
            .map_err(|_| AmountError::Overflow(s.to_string()))
    }
}

impl From<u64> for Amount {
    fn from(value: u64) -> Self {
        Amount(value)
    }
}

impl From<Amount> for u64 {
    fn from(value: Amount) -> Self {
        value.0
    }
}

impl FromStr for Amount {
    type Err = AmountError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Amount::parse_decimal(s)
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Amount({})", self.0)
    }
}

impl Serialize for Amount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Amount::parse_decimal(&s).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        amount: Amount,
    }

    #[test]
    fn serializes_as_decimal_string() {
        let json = serde_json::to_string(&Holder {
            amount: Amount::from_u64(12345),
        })
        .expect("serialize");
        assert_eq!(json, r#"{"amount":"12345"}"#);
    }

    #[test]
    fn deserializes_from_decimal_string() {
        let holder: Holder = serde_json::from_str(r#"{"amount":"1000"}"#).expect("deserialize");
        assert_eq!(holder.amount, Amount::from_u64(1000));
    }

    #[test]
    fn rejects_json_numbers_and_junk() {
        assert!(serde_json::from_str::<Holder>(r#"{"amount":1000}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"amount":"-1"}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"amount":"1.5"}"#).is_err());
        assert!(serde_json::from_str::<Holder>(r#"{"amount":" 1"}"#).is_err());
    }

    #[test]
    fn max_u64_roundtrips() {
        let json = serde_json::to_string(&Amount::MAX).expect("serialize");
        assert_eq!(json, format!("\"{}\"", u64::MAX));
        let back: Amount = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, Amount::MAX);
        assert!(Amount::parse_decimal("18446744073709551616").is_err());
    }

    #[test]
    fn parse_errors_are_clear() {
        assert_eq!(Amount::parse_decimal(""), Err(AmountError::Empty));
        assert_eq!(
            Amount::parse_decimal("12a"),
            Err(AmountError::NotDecimal("12a".to_string()))
        );
        assert_eq!(
            Amount::parse_decimal("99999999999999999999"),
            Err(AmountError::Overflow("99999999999999999999".to_string()))
        );
        assert_eq!(Amount::parse_decimal("007"), Ok(Amount::from_u64(7)));
    }

    #[test]
    fn checked_arithmetic() {
        let a = Amount::from_u64(10);
        let b = Amount::from_u64(3);
        assert_eq!(a.checked_add(b), Some(Amount::from_u64(13)));
        assert_eq!(a.checked_sub(b), Some(Amount::from_u64(7)));
        assert_eq!(b.checked_sub(a), None);
        assert_eq!(Amount::MAX.checked_add(Amount::from_u64(1)), None);
        assert!(Amount::ZERO.is_zero());
    }

    #[test]
    fn display_and_parse() {
        let a: Amount = "42".parse().expect("parse");
        assert_eq!(a.to_string(), "42");
        assert_eq!(a.as_u64(), 42);
    }
}
