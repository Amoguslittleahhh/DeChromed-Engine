//! Roadmap phase: Track D1 (URL parsing) through D3 (resource loading).
//! Per ROADMAP.md's "Language & repo shape", TLS will always be `rustls`,
//! never hand-rolled -- that's a standing exception, not a placeholder to
//! fill in later.

/// TODO(D1): replace with a real WHATWG URL Standard parser.
/// Currently a byte-for-byte passthrough with no validation, normalization,
/// or IDNA handling -- deliberately wrong, not yet attempted.
pub fn parse_url(input: &str) -> String {
    input.to_string()
}
