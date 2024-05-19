use bitflags::{bitflags, Flags};
use std::fmt::{Debug, Formatter};

use chrono::{DateTime, FixedOffset, Local, NaiveDateTime};
use ordered_float::OrderedFloat;

use crate::state_indicator::ComputedIndicatorState;

bitflags! {
    #[derive(Default, Debug, Copy, Clone, PartialEq)]
    pub struct PatternSimilarity: u8 {
        const RELPOS = 0b00000001;
        const RELMAG = 0b00000010;
        const TREND = 0b00000100;
        const VOLAT = 0b00001000;
        const FULL = Self::RELPOS.bits() | Self::RELMAG.bits() | Self::TREND.bits() | Self::VOLAT.bits();
    }
}

/// A pattern is a sequence of values that can be used to identify a specific
/// market condition. For example, a pattern could be a sequence of values
/// that are increasing in magnitude and position, indicating a strong uptrend.
///
/// Patterns are encoded into a single 64-bit integer, which is then used to
/// compare against other patterns. The encoding is done by using the following
/// 4 patterns:
///
/// 1. Relative position
/// 2. Relative magnitude
/// 3. Trend
/// 4. Volatility
///
/// Each of these patterns is encoded into a 16-bit integer, and then combined
/// into a single 64-bit integer. The 16-bit integers are encoded as follows:
///
/// 1. Relative position: 1 if the current value is greater than the previous value, 0 otherwise.
/// 2. Relative magnitude: 1 if the current change is greater than the previous change, 0 otherwise.
/// 3. Trend: 1 if the current value is greater than the average of the previous values, 0 otherwise.
/// 4. Volatility: 1 if the current value is greater than the mean + standard deviation of the previous values, 0 otherwise.
///
///
/// TODO:
/// Would be really nice to show some markers or something on the chart to
/// indicate where and which the the patterns are matched (or even partially matched).
///
/// And it would be also very useful to select areas on the chart to mark as
/// patterns, and then see how many times they are matched in the chart. This
/// would be a great way to find patterns that are not obvious. For example,
/// you could select a range of values that are increasing in magnitude and
/// position, and then see how many times that pattern is matched in the chart.
///
/// Highlighting areas of interest on the chart would be a great way to find
/// patterns that are not obvious.

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PatternMatch {
    pub p1:         Pattern,
    pub p2:         Pattern,
    pub similarity: PatternSimilarity,
}

impl PatternMatch {
    pub fn is_full_match(&self) -> bool {
        self.similarity == PatternSimilarity::FULL
    }

    pub fn is_partial_match(&self) -> bool {
        self.similarity != PatternSimilarity::FULL && self.similarity != PatternSimilarity::empty()
    }

    pub fn is_zero_match(&self) -> bool {
        self.similarity == PatternSimilarity::empty()
    }

    pub fn is_match(&self) -> bool {
        self.is_partial_match() || self.is_full_match()
    }

    pub fn matches_by(&self, similarity: PatternSimilarity) -> bool {
        self.similarity == similarity
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Pattern(u64);

impl Pattern {
    fn encode_relative_position(n: usize, values: &[f64]) -> u64 {
        let mut p = 0u64;
        for i in 0..n - 1 {
            if values[i + 1] > values[i] {
                p |= 1 << i;
            }
        }
        p
    }

    fn encode_relative_magnitude(n: usize, values: &[f64]) -> u64 {
        let mut p = 0u64;
        for i in 0..n - 2 {
            let current_change = values[i + 2] - values[i + 1];
            let previous_change = values[i + 1] - values[i];
            if current_change.abs() > previous_change.abs() {
                p |= 1 << i;
            }
        }
        p
    }

    fn encode_trend(n: usize, values: &[f64]) -> u64 {
        let mut p = 0u64;

        for i in 0..values.len() - n {
            let sum = values[i..i + n].iter().sum::<f64>();
            let avg = sum / n as f64;
            if values[i + n] > avg {
                p |= 1 << i;
            }
        }

        p
    }

    fn encode_volatility(n: usize, values: &[f64]) -> u64 {
        let mut p = 0u64;

        for i in 0..values.len() - n {
            let mean = values[i..i + n].iter().sum::<f64>() / n as f64;
            let var = values[i..i + n].iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / n as f64;
            let sd = var.sqrt();

            if values[i + n] > mean + sd || values[i + n] < mean - sd {
                p |= 1 << i;
            }
        }

        p
    }

    pub fn encode(n: usize, values: &[f64]) -> Self {
        let mut p = 0u64;

        let p1 = Self::encode_relative_position(n, values);
        let p2 = Self::encode_relative_magnitude(n, values);
        let p3 = Self::encode_trend(n, values);
        let p4 = Self::encode_volatility(n, values);

        // Combine the patterns into a single 64-bit integer
        Pattern(p1 | (p2 << 16) | (p3 << 32) | (p4 << 48))
    }

    pub fn match_with(&self, other: Pattern) -> PatternMatch {
        let mut similarity = PatternSimilarity::empty();

        let mask = 0xFFFF; // A mask to isolate 16 bits

        if (self.0 & mask) & (other.0 & mask) == (self.0 & mask) {
            similarity |= PatternSimilarity::RELPOS;
        }

        if ((self.0 >> 16) & mask) & ((other.0 >> 16) & mask) == ((self.0 >> 16) & mask) {
            similarity |= PatternSimilarity::RELMAG;
        }

        if ((self.0 >> 32) & mask) & ((other.0 >> 32) & mask) == ((self.0 >> 32) & mask) {
            similarity |= PatternSimilarity::TREND;
        }

        if ((self.0 >> 48) & mask) & ((other.0 >> 48) & mask) == ((self.0 >> 48) & mask) {
            similarity |= PatternSimilarity::VOLAT;
        }

        PatternMatch {
            p1: *self,
            p2: other,
            similarity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_match_full() {
        let p1 = Pattern((0b1100 << 48) | (0b1100 << 32) | (0b1100 << 16) | 0b1100);
        let p2 = Pattern((0b1100 << 48) | (0b1100 << 32) | (0b1100 << 16) | 0b1100);
        let match_ = p1.match_with(p2);

        assert!(match_.is_full_match());
        assert!(!match_.is_partial_match());
        assert!(!match_.is_zero_match());
        assert!(match_.is_match());
    }

    #[test]
    fn test_pattern_match_partial() {
        let p1 = Pattern((0b1100 << 48) | (0b1100 << 32) | (0b1100 << 16) | 0b1100);
        let p2 = Pattern((0b1000 << 48) | (0b1100 << 32) | (0b0100 << 16) | 0b1100);
        let match_ = p1.match_with(p2);

        assert!(!match_.is_full_match());
        assert!(match_.is_partial_match());
        assert!(!match_.is_zero_match());
        assert!(match_.is_match());
    }

    #[test]
    fn test_pattern_match_zero() {
        let p1 = Pattern((0b1100 << 48) | (0b1100 << 32) | (0b1100 << 16) | 0b1100);
        let p2 = Pattern((0b0011 << 48) | (0b0011 << 32) | (0b0011 << 16) | 0b0011);
        let match_ = p1.match_with(p2);

        assert!(!match_.is_full_match());
        assert!(!match_.is_partial_match());
        assert!(match_.is_zero_match());
        assert!(!match_.is_match());
    }
}
