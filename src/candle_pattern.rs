use crate::{
    candle::Candle,
    f,
    types::OrderedValueType,
    util::{calc_relative_change, is_between_inclusive},
};
use dyn_clonable::clonable;
use ordered_float::OrderedFloat;
use yata::core::ValueType;

impl Candle {
    // ----------------------------------------------------------------------------
    // One-candle patterns
    // ----------------------------------------------------------------------------

    /// Open and close prices are very close to each other, indicating indecision.
    pub fn is_doji<T>(&self, threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        calc_relative_change(self.open, self.close).abs() <= threshold.into()
    }

    /// Small body at the upper end (Hammer) with a long lower shadow.
    pub fn is_hammer<T>(&self, body_threshold: T, shadow_ratio: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_size = calc_relative_change(self.open, self.close).abs();
        let lower_shadow = self.lower_shadow();
        let upper_shadow = self.upper_shadow();
        body_size <= body_threshold.into() && lower_shadow / body_size >= shadow_ratio.into() && upper_shadow < lower_shadow
    }

    ///  Small body at the lower end (Inverted Hammer), with a long upper shadow.
    pub fn is_inverted_hammer<T>(&self, body_threshold: T, shadow_ratio: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_size = calc_relative_change(self.open, self.close).abs();
        let lower_shadow = self.lower_shadow();
        let upper_shadow = self.upper_shadow();
        body_size <= body_threshold.into() && upper_shadow / body_size >= shadow_ratio.into() && lower_shadow < upper_shadow
    }

    /// A long body with little to no shadows, indicating strong buying or selling pressure.
    pub fn is_marubozu<T>(&self, shadow_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let lower_shadow = self.lower_shadow();
        let upper_shadow = self.upper_shadow();
        upper_shadow <= shadow_threshold.into() && lower_shadow <= shadow_threshold.into()
    }

    /// Small body with longer upper and lower shadows, indicating indecision.
    pub fn is_spinning_top<T>(&self, body_threshold: T, shadow_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let shadow_threshold = shadow_threshold.into();
        let body_size = calc_relative_change(self.open, self.close).abs();
        let lower_shadow = self.lower_shadow();
        let upper_shadow = self.upper_shadow();
        body_size <= body_threshold && upper_shadow >= shadow_threshold && lower_shadow >= shadow_threshold
    }

    /// Appears in an uptrend, with a small body at the top and a long lower shadow, indicating a potential bearish reversal.
    pub fn is_hanging_man<T>(&self, body_threshold: T, shadow_ratio: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let shadow_ratio = shadow_ratio.into();
        let body_size = calc_relative_change(self.open, self.close).abs();
        let lower_shadow = self.lower_shadow();
        let upper_shadow = self.upper_shadow();
        body_size <= body_threshold && lower_shadow / body_size >= shadow_ratio && upper_shadow < lower_shadow
    }

    /// Appears in an uptrend, with a small body at the bottom and a long upper shadow, suggesting a bearish reversal.
    pub fn is_shooting_star<T>(&self, body_threshold: T, shadow_ratio: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let shadow_ratio = shadow_ratio.into();
        let body_size = calc_relative_change(self.open, self.close).abs();
        let lower_shadow = self.lower_shadow();
        let upper_shadow = self.upper_shadow();
        body_size <= body_threshold && upper_shadow / body_size >= shadow_ratio && lower_shadow < upper_shadow
    }

    // ----------------------------------------------------------------------------
    // Two-candle patterns
    // ----------------------------------------------------------------------------

    /// Checks for a Bullish Engulfing pattern. This pattern occurs when a small red candle
    /// is followed by a larger green candle, indicating a potential bullish reversal.
    pub fn is_bullish_engulfing<T>(&self, previous: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        previous.is_red()
            && self.is_green()
            && calc_relative_change(previous.open, previous.close).abs() <= body_threshold
            && self.close > previous.open
            && self.open < previous.close
    }

    /// Checks for a Bearish Engulfing pattern. This pattern is identified when a small green candle
    /// is followed by a larger red candle, signaling a potential bearish reversal.
    pub fn is_bearish_engulfing<T>(&self, previous: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        previous.is_green()
            && self.is_red()
            && calc_relative_change(previous.open, previous.close).abs() <= body_threshold
            && self.close < previous.open
            && self.open > previous.close
    }

    /// Checks if the current candle forms a Tweezer Tops pattern with the previous candle.
    /// A bearish reversal pattern characterized by two candles with matching highs.
    /// The first (previous) is red, and the second (current) is green.
    pub fn is_tweezer_tops<T>(&self, previous: &Candle, threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let threshold = threshold.into();
        is_between_inclusive(previous.high, self.high - threshold, self.high + threshold) && previous.is_red() && self.is_green()
    }

    /// Checks if the current candle forms a Tweezer Bottoms pattern with the previous candle.
    /// A bullish reversal pattern with two candles having matching lows.
    /// The first (previous) is green, and the second (current) is red.
    pub fn is_tweezer_bottoms<T>(&self, previous: &Candle, threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let threshold = threshold.into();
        is_between_inclusive(previous.low, self.low - threshold, self.low + threshold) && previous.is_green() && self.is_red()
    }

    /// Checks for a Piercing Line, a bullish reversal pattern indicated by a long red candle
    /// followed by a long green candle. The green candle opens below the red's close and closes above
    /// its midpoint.
    pub fn is_piercing_line<T>(&self, previous: &Candle, penetration_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let penetration_threshold = penetration_threshold.into();
        previous.is_red()
            && self.is_green()
            && self.open < previous.close
            && self.close > (previous.open + (previous.open - previous.close) * penetration_threshold)
    }

    /// Checks for Dark Cloud Cover, a bearish reversal pattern characterized by a long green candle
    /// followed by a long red candle. The red candle opens above the green's close and closes below
    /// its midpoint.
    pub fn is_dark_cloud_cover<T>(&self, previous: &Candle, penetration_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let penetration_threshold = penetration_threshold.into();
        previous.is_green()
            && self.is_red()
            && self.open > previous.close
            && self.close < (previous.open + (previous.close - previous.open) * penetration_threshold)
    }

    // ----------------------------------------------------------------------------
    // Three-candle patterns
    // ----------------------------------------------------------------------------

    /*
    /// Checks for a Morning Star pattern, indicating a bullish reversal.
    /// This candle is the middle of the pattern.
    pub fn is_morning_star<T>(&self, prev: &Candle, next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        prev.is_red()
            && calc_relative_change(prev.open, prev.close).abs() > body_threshold
            && prev.close < self.open
            && self.close < next.open
            && next.is_green()
            && calc_relative_change(next.open, next.close).abs() > body_threshold
    }
     */

    pub fn is_morning_star<T>(&self, prev: &Candle, next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        prev.is_red() && self.is_green() && self.open < prev.close && self.close > (prev.open + (prev.open - prev.close) * body_threshold)
    }

    /*
    pub fn is_morning_star<T>(&self, prev: &Candle, next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let prev_is_red = prev.is_red();
        let prev_body_size = calc_relative_change(prev.open, prev.close).abs();
        let prev_condition = prev_body_size > body_threshold;
        let current_condition = prev.close < self.open && self.close < next.open;
        let next_is_green = next.is_green();
        let next_body_size = calc_relative_change(next.open, next.close).abs();
        let next_condition = next_body_size > body_threshold;

        println!("prev_is_red: {}", prev_is_red);
        println!("prev_body_size: {}", prev_body_size);
        println!("prev_condition: {}", prev_condition);
        println!("current_condition: {}", current_condition);
        println!("next_is_green: {}", next_is_green);
        println!("next_body_size: {}", next_body_size);
        println!("next_condition: {}", next_condition);

        prev_is_red && prev_condition && current_condition && next_is_green && next_condition
    }
     */

    /*
    pub fn is_morning_star<T>(&self, prev: &Candle, next: &Candle, body_threshold: T) -> bool
        where
            T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        eprintln!("body_threshold (before .into()) = {:#?}", body_threshold);
        let prev_is_red = prev.is_red();
        let prev_body_size = (prev.open - prev.close).abs();
        let prev_condition = prev_body_size > body_threshold;
        let current_condition = self.close < next.open; // Relaxed condition
        let next_is_green = next.is_green();
        let next_body_size = (next.open - next.close).abs();
        let next_condition = next_body_size > body_threshold;

        eprintln!("self = {:#?}", self);
        eprintln!("prev = {:#?}", prev);
        eprintln!("next = {:#?}", next);
        eprintln!("body_threshold (after .into()) = {:#?}", body_threshold);
        eprintln!("prev_is_red = {:#?}", prev_is_red);
        eprintln!("prev_body_size = {:#?}", prev_body_size);
        eprintln!("prev_condition = {:#?}", prev_condition);
        eprintln!("current_condition = {:#?}", current_condition);
        eprintln!("next_is_green = {:#?}", next_is_green);
        eprintln!("next_body_size = {:#?}", next_body_size);
        eprintln!("next_condition = {:#?}", next_condition);
        eprintln!(
            "prev_is_red && prev_condition && current_condition && next_is_green && next_condition = {:#?}",
            prev_is_red && prev_condition && current_condition && next_is_green && next_condition
        );

        prev_is_red && prev_condition && current_condition && next_is_green && next_condition
    }
     */

    /// Checks for an Evening Star pattern, indicating a bearish reversal.
    /// This candle is the middle of the pattern.
    pub fn is_evening_star<T>(&self, prev: &Candle, next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        prev.is_green()
            && calc_relative_change(prev.open, prev.close).abs() > body_threshold
            && prev.close > self.open
            && self.close > next.open
            && next.is_red()
            && calc_relative_change(next.open, next.close).abs() > body_threshold
    }

    /// Checks if this candle and the next two form a Three White Soldiers pattern.
    /// A bullish pattern with three consecutive long green candles.
    pub fn is_three_white_soldiers<T>(&self, next: &Candle, next_next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        self.is_green()
            && next.is_green()
            && next_next.is_green()
            && calc_relative_change(self.open, self.close).abs() > body_threshold
            && calc_relative_change(next.open, next.close).abs() > body_threshold
            && calc_relative_change(next_next.open, next_next.close).abs() > body_threshold
            && next.close > self.close
            && next_next.close > next.close
    }

    /// Checks if this candle and the next two form a Three Black Crows pattern.
    /// A bearish pattern with three consecutive long red candles.
    pub fn is_three_black_crows<T>(&self, next: &Candle, next_next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        self.is_red()
            && next.is_red()
            && next_next.is_red()
            && calc_relative_change(self.open, self.close).abs() > body_threshold
            && calc_relative_change(next.open, next.close).abs() > body_threshold
            && calc_relative_change(next_next.open, next_next.close).abs() > body_threshold
            && next.close < self.close
            && next_next.close < next.close
    }

    /// Checks for an Abandoned Baby pattern, indicating a potential reversal.
    /// First candle is a long body, second is a Doji gapping above or below the first,
    /// and third is a candle of the opposite color.
    pub fn is_abandoned_baby<T>(&self, prev: &Candle, next: &Candle, body_threshold: T, doji_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let doji_threshold = doji_threshold.into();
        let is_prev_long = calc_relative_change(prev.open, prev.close).abs() > body_threshold;
        let is_self_doji = self.is_doji(doji_threshold);
        let is_next_opposite_color = (prev.is_red() && next.is_green()) || (prev.is_green() && next.is_red());

        is_prev_long
            && is_self_doji
            && is_next_opposite_color
            && ((prev.is_red() && self.low > prev.high && next.open < self.close) || (prev.is_green() && self.high < prev.low && next.close > self.open))
    }

    /// Checks for an Evening/Morning Doji Star, indicating a bearish/bullish reversal.
    /// First candle is a long body, second is a Doji, and third is a long candle of opposite color.
    pub fn is_evening_morning_doji_star<T>(&self, prev: &Candle, next: &Candle, body_threshold: T, doji_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let doji_threshold = doji_threshold.into();
        let is_prev_long = calc_relative_change(prev.open, prev.close).abs() > body_threshold;
        let is_self_doji = self.is_doji(doji_threshold);
        let is_next_opposite_color = (prev.is_red() && next.is_green()) || (prev.is_green() && next.is_red());

        is_prev_long
            && is_self_doji
            && is_next_opposite_color
            && ((prev.is_red() && next.is_green() && self.low > prev.high && next.open < self.close)
                || (prev.is_green() && next.is_red() && self.high < prev.low && next.close > self.open))
    }

    /// A Doji Star pattern occurs when a Doji forms after a long
    /// candle (either green or red). It signals a potential reversal.
    pub fn is_general_doji_star(&self, next: &Candle, doji_threshold: ValueType) -> bool {
        (self.is_green() || self.is_red()) && next.is_doji(doji_threshold)
    }

    /// Checks for Three Inside Up/Down pattern on the current candle.
    pub fn is_three_inside<T>(&self, prev: &Candle, next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        let is_prev_long = calc_relative_change(prev.open, prev.close).abs() > body_threshold;
        let is_self_inside = self.is_inside(prev);
        let is_next_bullish_or_bearish = (prev.is_red() && next.close > prev.close) || (prev.is_green() && next.close < prev.close);

        is_prev_long && is_self_inside && is_next_bullish_or_bearish
    }

    /// Checks if this candle, along with the next two, forms a Three Outside Up pattern.
    pub fn is_three_outside_up<T>(&self, next: &Candle, next_next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        self.is_bullish_engulfing(&next, body_threshold) && next_next.is_green() && next_next.close > next.close
    }

    /// Checks if this candle, along with the next two, forms a Three Outside Down pattern.
    pub fn is_three_outside_down<T>(&self, next: &Candle, next_next: &Candle, body_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let body_threshold = body_threshold.into();
        self.is_bearish_engulfing(&next, body_threshold) && next_next.is_red() && next_next.close < next.close
    }

    /// A bullish reversal pattern consisting of three consecutive long green candles with short shadows.
    /// Each candle opens within the body of the previous candle.
    pub fn is_three_advancing_white_soldiers<T>(&self, next: &Candle, next_next: &Candle, shadow_threshold: T) -> bool
    where
        T: Clone + Copy + Into<ValueType>,
    {
        let shadow_threshold = shadow_threshold.into();
        self.is_green()
            && next.is_green()
            && next_next.is_green()
            && self.upper_shadow() < shadow_threshold
            && next.upper_shadow() < shadow_threshold
            && next_next.upper_shadow() < shadow_threshold
            && next.open > self.open
            && next.close > self.close
            && next_next.open > next.open
            && next_next.close > next.close
    }

    /// The Rising Three Methods consists of a long green candle, followed by
    /// three smaller red candles within its range, and then another long green candle.
    pub fn is_rising_three_methods(&self, next: [&Candle; 4]) -> bool {
        self.is_green()
            && next[0].is_red()
            && next[0].high < self.high
            && next[0].low > self.low
            && next[1].is_red()
            && next[1].high < self.high
            && next[1].low > self.low
            && next[2].is_red()
            && next[2].high < self.high
            && next[2].low > self.low
            && next[3].is_green()
            && next[3].close > self.high
    }

    /// The Falling Three Methods is the opposite, with a long red candle, three
    /// smaller green candles, and another long red candle.
    pub fn is_falling_three_methods(&self, next: [&Candle; 4]) -> bool {
        self.is_red()
            && next[0].is_green()
            && next[0].high < self.high
            && next[0].low > self.low
            && next[1].is_green()
            && next[1].high < self.high
            && next[1].low > self.low
            && next[2].is_green()
            && next[2].high < self.high
            && next[2].low > self.low
            && next[3].is_red()
            && next[3].close < self.low
    }

    /// Similar to a Harami pattern, but the second candle is a Doji.
    /// It's considered a more significant reversal signal than a standard Harami.
    /// Checks if the current candle forms a Harami Cross pattern.
    pub fn is_harami_cross(&self, next: &Candle, doji_threshold: ValueType) -> bool {
        (self.is_green() && next.is_doji(doji_threshold) && next.open > self.low && next.close < self.high)
            || (self.is_red() && next.is_doji(doji_threshold) && next.open < self.high && next.close > self.low)
    }

    /// This pattern consists of two similar candles (either green or red)
    /// with a different candle sandwiched between them. It's a reversal pattern.
    pub fn is_stick_sandwich(&self, next: &Candle, next_next: &Candle) -> bool {
        self.color() == next_next.color() && self.color() != next.color() && self.close == next_next.close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candle::CandleColor;

    fn create_ohlc(open: ValueType, high: ValueType, low: ValueType, close: ValueType) -> Candle {
        Candle {
            symbol: Default::default(),
            timeframe: Default::default(),
            open_time: Default::default(),
            close_time: Default::default(),
            open,
            close,
            volume: 0.0,
            number_of_trades: 0,
            quote_asset_volume: 0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume: 0.0,
            high,
            low,
            is_final: false,
        }
    }

    #[test]
    fn test_color() {
        let green_candle = create_ohlc(100.0, 115.0, 95.0, 110.0);
        let red_candle = create_ohlc(110.0, 115.0, 95.0, 100.0);

        assert_eq!(green_candle.color(), CandleColor::Green);
        assert_eq!(red_candle.color(), CandleColor::Red);
    }

    #[test]
    fn test_is_green() {
        let green_candle = create_ohlc(100.0, 115.0, 95.0, 110.0);
        let red_candle = create_ohlc(110.0, 115.0, 95.0, 100.0);

        assert!(green_candle.is_green());
        assert!(!red_candle.is_green());
    }

    #[test]
    fn test_color_matches() {
        let green_candle = create_ohlc(100.0, 115.0, 95.0, 110.0);
        let red_candle = create_ohlc(110.0, 115.0, 95.0, 100.0);

        assert!(green_candle.color_matches(&green_candle));
        assert!(red_candle.color_matches(&red_candle));
        assert!(!green_candle.color_matches(&red_candle));
        assert!(!red_candle.color_matches(&green_candle));
    }

    #[test]
    fn test_is_doji_positive() {
        let candle = create_ohlc(100.0, 105.0, 95.0, 100.05);
        assert!(candle.is_doji(0.1));
    }

    #[test]
    fn test_is_doji_negative() {
        let candle = create_ohlc(100.0, 105.0, 95.0, 102.0);
        assert!(!candle.is_doji(0.01));
    }

    #[test]
    fn test_is_doji_edge_case() {
        let candle = create_ohlc(100.0, 105.0, 95.0, 100.1);
        assert!(candle.is_doji(0.1));
    }

    #[test]
    fn test_is_hammer_positive() {
        let candle = create_ohlc(100.0, 101.0, 90.0, 100.5);
        assert!(candle.is_hammer(0.1, 2.0));
    }

    #[test]
    fn test_is_hammer_negative() {
        let candle = create_ohlc(100.0, 115.0, 95.0, 110.0);
        assert!(!candle.is_hammer(0.05, 3.0));
    }

    #[test]
    fn test_is_inverted_hammer_positive() {
        let candle = create_ohlc(100.0, 110.0, 99.0, 100.5);
        assert!(candle.is_inverted_hammer(0.1, 2.0));
    }

    #[test]
    fn test_is_inverted_hammer_edge_case() {
        let candle = create_ohlc(1000.0, 1100.0, 990.0, 1000.9);
        assert!(candle.is_inverted_hammer(0.1, 2.0));
    }

    #[test]
    fn test_is_inverted_hammer_negative() {
        let candle = create_ohlc(100.0, 115.0, 95.0, 110.0);
        assert!(!candle.is_inverted_hammer(0.05, 3.0));
    }

    #[test]
    fn test_is_marubozu_positive() {
        let candle = create_ohlc(100.0, 110.05, 99.95, 110.0);
        assert!(candle.is_marubozu(0.05));
    }

    #[test]
    fn test_is_marubozu_negative() {
        let candle = create_ohlc(100.0, 115.0, 95.0, 110.0);
        assert!(!candle.is_marubozu(0.05));
    }

    #[test]
    fn test_is_bullish_engulfing_positive() {
        let prev_candle = create_ohlc(100.0, 101.0, 94.0, 95.0); // Red candle
        let current_candle = create_ohlc(94.0, 106.0, 93.0, 105.0); // Green candle engulfing the previous
        assert!(current_candle.is_bullish_engulfing(&prev_candle, 0.1));
    }

    #[test]
    fn test_is_bullish_engulfing_negative() {
        let prev_candle = create_ohlc(100.0, 106.0, 99.0, 105.0); // Green candle
        let current_candle = create_ohlc(105.0, 107.0, 104.0, 106.0); // Another green candle
        assert!(!current_candle.is_bullish_engulfing(&prev_candle, 0.1));
    }

    #[test]
    fn test_is_bearish_engulfing_positive() {
        let prev_candle = create_ohlc(100.0, 106.0, 99.0, 100.1); // Smaller body green candle
        let current_candle = create_ohlc(105.0, 107.0, 94.0, 95.0); // Red candle engulfing the previous
        assert!(current_candle.is_bearish_engulfing(&prev_candle, 0.1));
    }

    #[test]
    fn test_is_bearish_engulfing_negative() {
        let prev_candle = create_ohlc(100.0, 101.0, 94.0, 95.0); // Red candle
        let current_candle = create_ohlc(95.0, 97.0, 94.0, 96.0); // Smaller red candle
        assert!(!current_candle.is_bearish_engulfing(&prev_candle, 0.1));
    }

    #[test]
    fn test_is_tweezer_tops_positive() {
        let prev_candle = create_ohlc(105.0, 110.0, 95.0, 100.0); // Red candle
        let current_candle = create_ohlc(100.0, 110.0, 95.0, 105.0); // Green candle, matching high
        assert!(current_candle.is_tweezer_tops(&prev_candle, 0.1));
    }

    #[test]
    fn test_is_tweezer_tops_negative() {
        let prev_candle = create_ohlc(105.0, 110.0, 95.0, 100.0); // Red candle
        let current_candle = create_ohlc(100.0, 109.0, 95.0, 105.0); // Green candle, different high
        assert!(!current_candle.is_tweezer_tops(&prev_candle, 0.1));
    }

    #[test]
    fn test_is_piercing_line_positive() {
        let prev_candle = create_ohlc(105.0, 106.0, 94.0, 95.0); // Long red candle
        let current_candle = create_ohlc(94.0, 111.0, 93.0, 110.5); // Long green candle
        assert!(current_candle.is_piercing_line(&prev_candle, 0.5));
    }

    #[test]
    fn test_is_piercing_line_negative() {
        let prev_candle = create_ohlc(105.0, 106.0, 94.0, 95.0); // Long red candle
        let current_candle = create_ohlc(94.0, 98.0, 93.0, 97.0); // Short green candle
        assert!(!current_candle.is_piercing_line(&prev_candle, 0.5));
    }

    #[test]
    fn test_is_dark_cloud_cover_positive() {
        let prev_candle = create_ohlc(95.0, 106.0, 94.0, 105.0); // Long green candle
        let current_candle = create_ohlc(106.0, 107.0, 96.0, 97.0); // Long red candle
        assert!(current_candle.is_dark_cloud_cover(&prev_candle, 0.5));
    }

    #[test]
    fn test_is_dark_cloud_cover_negative() {
        let prev_candle = create_ohlc(95.0, 106.0, 94.0, 105.0); // Long green candle
        let current_candle = create_ohlc(106.0, 107.0, 99.0, 100.0); // Smaller red candle
        assert!(!current_candle.is_dark_cloud_cover(&prev_candle, 0.5));
    }

    /*
    #[test]
    fn test_is_morning_star_positive() {
        // Long red candle
        let prev_candle = create_ohlc(1200.0, 1250.0, 950.0, 1000.0);

        // Short candle, opens above the previous close and closes below the next open
        let current_candle = create_ohlc(1001.0, 1020.0, 990.0, 1010.0);

        // Long green candle
        let next_candle = create_ohlc(1011.0, 1300.0, 1000.0, 1250.0);

        assert!(current_candle.is_morning_star(&prev_candle, &next_candle, 50.0));
    }
     */

    #[test]
    fn test_is_morning_star_positive() {
        let prev_candle = Candle {
            open: 100.0,
            high: 105.0,
            low: 95.0,
            close: 95.0,
            volume: 1000.0,
            ..Default::default()
        };
        let current_candle = Candle {
            open: 90.0,
            high: 95.0,
            low: 85.0,
            close: 90.0,
            volume: 1000.0,
            ..Default::default()
        };
        let next_candle = Candle {
            open: 95.0,
            high: 110.0,
            low: 90.0,
            close: 110.0,
            volume: 1000.0,
            ..Default::default()
        };

        assert!(current_candle.is_morning_star(&prev_candle, &next_candle, 50.0));
    }

    #[test]
    fn test_is_morning_star_negative() {
        let prev_candle = create_ohlc(105.0, 106.0, 94.0, 95.0); // Long red candle
        let current_candle = create_ohlc(95.0, 97.0, 94.5, 96.0); // Short candle
        let next_candle = create_ohlc(96.0, 97.0, 94.0, 95.0); // Small red candle
        assert!(!current_candle.is_morning_star(&prev_candle, &next_candle, 0.1));
    }
}
