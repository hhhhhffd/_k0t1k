use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A parsed `java.version`: `1.8.0_392`, `17.0.15`, `21.0.7+6`, `25-ea`.
/// Build metadata (`+6`) is dropped; it does not take part in comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaVersion {
    pub major: u32,
    pub minor: u32,
    pub security: u32,
    pub prerelease: Option<String>,
}

impl FromStr for JavaVersion {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let invalid = || Error::Other(format!("unrecognised Java version: {text:?}"));

        let text = text.trim();
        let without_build = text.split('+').next().unwrap_or(text);
        let (numbers, prerelease) = match without_build.split_once('-') {
            Some((numbers, pre)) => (numbers, Some(pre.to_owned())),
            None => (without_build, None),
        };

        // Legacy scheme: 1.<major>.<minor>_<security>
        let numbers = numbers.strip_prefix("1.").unwrap_or(numbers).replace('_', ".");
        let mut parts = numbers.split('.').map(|part| part.parse::<u32>().map_err(|_| invalid()));
        let major = parts.next().ok_or_else(invalid)??;
        let minor = parts.next().transpose()?.unwrap_or(0);
        let security = parts.next().transpose()?.unwrap_or(0);

        Ok(Self {
            major,
            minor,
            security,
            prerelease,
        })
    }
}

impl fmt::Display for JavaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.major <= 8 {
            write!(f, "1.{}.{}_{}", self.major, self.minor, self.security)?;
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.security)?;
        }
        if let Some(pre) = &self.prerelease {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl Ord for JavaVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.security.cmp(&other.security))
            // A release is newer than any prerelease of the same number.
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for JavaVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> JavaVersion {
        text.parse().unwrap()
    }

    #[test]
    fn parses_legacy_and_modern_schemes() {
        let legacy = parse("1.8.0_392");
        assert_eq!((legacy.major, legacy.minor, legacy.security), (8, 0, 392));

        let modern = parse("17.0.15");
        assert_eq!((modern.major, modern.minor, modern.security), (17, 0, 15));

        let build = parse("21.0.7+6");
        assert_eq!((build.major, build.minor, build.security, build.prerelease), (21, 0, 7, None));

        let early_access = parse("25-ea");
        assert_eq!((early_access.major, early_access.minor), (25, 0));
        assert_eq!(early_access.prerelease.as_deref(), Some("ea"));

        assert_eq!(parse("1.8.0_51-b16").prerelease.as_deref(), Some("b16"));
        assert_eq!(parse("21").major, 21);
    }

    #[test]
    fn rejects_garbage() {
        assert!("".parse::<JavaVersion>().is_err());
        assert!("openjdk".parse::<JavaVersion>().is_err());
        assert!("17.x".parse::<JavaVersion>().is_err());
    }

    #[test]
    fn compares_numerically_and_prerelease_is_older() {
        assert!(parse("1.8.0_392") < parse("17.0.1"));
        assert!(parse("17.0.9") < parse("17.0.15"));
        assert!(parse("21.0.7") > parse("21.0.7-ea"));
        assert!(parse("25-ea") > parse("21.0.7"));
        assert_eq!(parse("21.0.7+6"), parse("21.0.7+9"));
    }

    #[test]
    fn display_roundtrips() {
        for text in ["1.8.0_392", "17.0.15", "25.0.0-ea"] {
            assert_eq!(parse(text).to_string(), text);
        }
    }
}
