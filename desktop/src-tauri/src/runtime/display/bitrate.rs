//! Benchmark controls never change the production bitrate policy.
pub(super) fn fixed_bitrate_requested(value: Option<&str>) -> bool {
    cfg!(any(debug_assertions, feature = "display-benchmark")) && value == Some("1")
}

#[cfg(test)]
mod tests {
    use super::fixed_bitrate_requested;

    #[test]
    fn fixed_bitrate_defaults_to_adaptive() {
        for value in [
            None,
            Some("0"),
            Some(""),
            Some("true"),
            Some(" 1"),
            Some("2"),
        ] {
            assert!(
                !fixed_bitrate_requested(value),
                "unexpected option: {value:?}"
            );
        }
    }

    #[test]
    fn fixed_bitrate_requires_benchmark_or_debug_build() {
        assert_eq!(
            fixed_bitrate_requested(Some("1")),
            cfg!(any(debug_assertions, feature = "display-benchmark"))
        );
    }
}
