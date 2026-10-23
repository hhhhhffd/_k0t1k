use std::cmp::Ordering;

fn segments(version: &str) -> Vec<&str> {
    version.split(|c: char| !c.is_ascii_alphanumeric()).filter(|segment| !segment.is_empty()).collect()
}

fn is_numeric(segment: &str) -> bool {
    segment.chars().all(|c| c.is_ascii_digit())
}

fn compare_segments(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        (Ok(_), Err(_)) => Ordering::Greater,
        (Err(_), Ok(_)) => Ordering::Less,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

/// Orders dotted versions segment by segment, numbers numerically. When one version is a prefix
/// of the other, the longer one is newer unless its extra part is a qualifier (`3.3.3-rc1`).
pub(crate) fn compare_versions(a: &str, b: &str) -> Ordering {
    let (a, b) = (segments(a), segments(b));
    for (a, b) in a.iter().zip(&b) {
        let ordering = compare_segments(a, b);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    match a.len().cmp(&b.len()) {
        Ordering::Equal => Ordering::Equal,
        Ordering::Greater if is_numeric(a[b.len()]) => Ordering::Greater,
        Ordering::Greater => Ordering::Less,
        Ordering::Less if is_numeric(b[a.len()]) => Ordering::Less,
        Ordering::Less => Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_versions() {
        assert_eq!(compare_versions("3.3.3", "3.3.1"), Ordering::Greater);
        assert_eq!(compare_versions("3.10.0", "3.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("2.9.1", "2.9.4-nightly-20150209"), Ordering::Less);
        assert_eq!(compare_versions("1.2", "1.2.0"), Ordering::Less);
        assert_eq!(compare_versions("3.3.3-rc1", "3.3.3"), Ordering::Less);
        assert_eq!(compare_versions("9.7.1", "9.7.1"), Ordering::Equal);
    }
}
