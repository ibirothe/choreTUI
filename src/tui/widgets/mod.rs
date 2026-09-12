//! Shared terminal widgets.

/// Truncate text by Unicode scalar count and retain an explicit ellipsis.
#[must_use]
pub fn truncate_with_ellipsis(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_owned();
    }
    value.chars().take(width - 1).chain(['…']).collect()
}

#[cfg(test)]
mod tests {
    use super::truncate_with_ellipsis;

    #[test]
    fn truncation_preserves_markers_space_and_unicode_boundaries() {
        assert_eq!(truncate_with_ellipsis("Laundry", 7), "Laundry");
        assert_eq!(truncate_with_ellipsis("Laundry", 5), "Laun…");
        assert_eq!(truncate_with_ellipsis("🧺Laundry", 3), "🧺L…");
        assert_eq!(truncate_with_ellipsis("Laundry", 1), "…");
        assert_eq!(truncate_with_ellipsis("Laundry", 0), "");
    }
}
