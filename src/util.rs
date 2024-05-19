use std::{
    cmp::Ordering,
    fmt::{Display, Formatter},
    hash::{BuildHasher, Hasher},
    ops::{DerefMut, Neg, RangeInclusive},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::{
    candle::Candle,
    candle_series::ComputedState,
    config::{Config, TimeframeConfig},
    model::{ExchangeInformation, Symbol},
    timeframe::Timeframe,
    timeset::TimeSet,
    types::{Direction, Gauge, Motion, NormalizedSide, OrderedValueType, PriceDistribution, QuantityDistribution},
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use egui::TextBuffer;
use itertools::Itertools;
use murmur3::murmur3_32;
use num::Integer;
use num_traits::{real::Real, Float, Num, NumOps, Unsigned, Zero};
use ordered_float::OrderedFloat;
use rand::{rngs::OsRng, Rng};
use ustr::Ustr;
use yata::core::{Method, ValueType, OHLCV};

// found this useful macro somewhere on the web
#[macro_export]
macro_rules! show_size {
    (header) => {
        println!("{:<22} {:>4}    {}", "Type", "T", "Option<T>");
    };
    ($t:ty) => {
        println!("{:<22} {:4} {:4}", stringify!($t), size_of::<$t>(), size_of::<Option<$t>>())
    };
}

/// Fibonacci sequence for moving averages periods (used for support and resistance)
pub const FIB_SEQUENCE: [usize; 28] = [
    3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946, 17711, 28657, 46368, 75025, 121393, 196418, 317811, 514229, 832040,
    1346269,
];

pub const MINS_PER_DAY: f64 = 24.0 * 60.0;
pub const MINS_PER_HOUR: f64 = 60.0;

pub fn new_id_u64() -> u64 {
    OsRng.gen_range(1..1024000000)
}

#[inline]
pub fn hash_id_u32(id: &str) -> u32 {
    murmur3_32(&mut id.as_bytes(), 0).expect("Failed to hash id")
}

#[inline]
pub fn hash_id_u64(id: &str) -> u64 {
    murmur3_32(&mut id.as_bytes(), 0).expect("Failed to hash id") as u64
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub coin_id:                u64,
    pub price:                  ValueType,
    pub state:                  TimeSet<ComputedState>,
    pub total_computation_time: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Coin,
    CoinPair,
    Trade,
    Order,
    Stoploss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    pub kind: EntityKind,
    pub id:   u64,
}

impl Entity {
    pub fn new(kind: EntityKind, id: u64) -> Self {
        Self { kind, id }
    }

    pub fn coin(id: u64) -> Self {
        Self::new(EntityKind::Coin, id)
    }

    pub fn coin_pair(id: u64) -> Self {
        Self::new(EntityKind::CoinPair, id)
    }

    pub fn trade(id: u64) -> Self {
        Self::new(EntityKind::Trade, id)
    }

    pub fn order(id: u64) -> Self {
        Self::new(EntityKind::Order, id)
    }

    pub fn stop_loss(id: u64) -> Self {
        Self::new(EntityKind::Stoploss, id)
    }

    pub fn kind(&self) -> EntityKind {
        self.kind
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Display for Entity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.id)
    }
}

struct KeyPressState {
    key:                    egui::Key,
    last_press_time:        Option<Instant>,
    double_press_threshold: Duration, // Threshold for double press
}

impl KeyPressState {
    fn new(key: egui::Key, double_press_threshold_ms: u64) -> Self {
        KeyPressState {
            key,
            last_press_time: None,
            double_press_threshold: Duration::from_millis(double_press_threshold_ms),
        }
    }

    fn check_double_press(&mut self, cx: &egui::Context, key: egui::Key) -> bool {
        let mut double_press = false;

        cx.input(|state| {
            if let Some(last_press) = self.last_press_time {
                self.last_press_time = Some(Instant::now());

                if last_press.elapsed() < self.double_press_threshold {
                    return true;
                }
            }

            false
        })
    }
}

pub fn gauge_location<T: Into<ValueType>>(loc: T) -> Gauge {
    let loc = loc.into();

    if is_between(loc, 1.00, 1000.0) {
        Gauge::BreakoutUp
    } else if is_between_inclusive(loc, 0.85, 1.00) {
        Gauge::Crest
    } else if is_between_inclusive(loc, 0.75, 0.85) {
        Gauge::VeryHigh
    } else if is_between_inclusive(loc, 0.55, 0.75) {
        Gauge::High
    } else if is_between_inclusive(loc, 0.51, 0.55) {
        Gauge::AbovePar
    } else if is_between_inclusive(loc, 0.49, 0.51) {
        Gauge::Parity
    } else if is_between_inclusive(loc, 0.45, 0.49) {
        Gauge::BelowPar
    } else if is_between_inclusive(loc, 0.25, 0.45) {
        Gauge::Low
    } else if is_between_inclusive(loc, 0.15, 0.25) {
        Gauge::VeryLow
    } else if is_between_inclusive(loc, 0.00, 0.15) {
        Gauge::Trough
    } else if is_between_inclusive(loc, -1000.0, 0.00) {
        Gauge::BreakoutDown
    } else {
        Gauge::Parity
    }
}

pub fn closed_candle(close: ValueType) -> Candle {
    Candle { close, ..Default::default() }
}

pub fn extract_symbols(exchange_information: ExchangeInformation, suffix: Option<&str>) -> Result<Vec<Symbol>> {
    let symbols: Vec<Symbol> = match suffix {
        None => exchange_information.symbols.into_iter().map(|s| s).collect(),
        Some(suffix) => exchange_information
            .symbols
            .into_iter()
            .filter_map(|s| if s.symbol.ends_with(suffix) { Some(s) } else { None })
            .collect(),
    };

    if symbols.is_empty() {
        if suffix.is_some() {
            Err(anyhow!("no symbols with suffix: {}", suffix.unwrap()))
        } else {
            Err(anyhow!("no symbols with suffix {}", suffix.unwrap()))
        }
    } else {
        Ok(symbols)
    }
}

#[inline]
pub fn s2f(s: &str) -> ValueType {
    fast_float::parse(s).unwrap()
}

#[inline]
pub fn s2of(s: &str) -> OrderedFloat<ValueType> {
    OrderedFloat(fast_float::parse(s).unwrap())
}

#[cfg(target_arch = "x86")]
#[inline]
pub fn s2i64(s: &str) -> i64 {
    atoi_simd::parse(s.as_bytes()).unwrap()
}

#[cfg(target_arch = "x86")]
#[inline]
pub fn s2u64(s: &str) -> Result<u64> {
    atoi_simd::parse(s.as_bytes()).map_err(|err| anyhow!(err.to_string()))
}

#[cfg(not(target_arch = "x86"))]
#[inline]
pub fn s2i64(s: &str) -> i64 {
    s.parse::<i64>().unwrap()
}

#[cfg(not(target_arch = "x86"))]
#[inline]
pub fn s2u64(s: &str) -> Result<u64> {
    s.parse::<u64>().map_err(|err| anyhow!(err.to_string()))
}

#[cfg(target_arch = "x86")]
#[inline]
pub fn s2i32(s: &str) -> i32 {
    atoi_simd::parse(s.as_bytes()).unwrap()
}

#[cfg(target_arch = "x86")]
#[inline]
pub fn s2u32(s: &str) -> Result<u32> {
    atoi_simd::parse(s.as_bytes()).map_err(|err| anyhow!(err.to_string()))
}

#[cfg(not(target_arch = "x86"))]
#[inline]
pub fn s2i32(s: &str) -> i32 {
    s.parse::<i32>().unwrap()
}

#[cfg(not(target_arch = "x86"))]
#[inline]
pub fn s2u32(s: &str) -> Result<u32> {
    s.parse::<u32>().map_err(|err| anyhow!(err.to_string()))
}

#[inline]
pub fn itoa_u64<T>(number: T) -> String
where
    T: Into<u64> + Copy,
{
    itoa::Buffer::new().format(number.into()).to_string()
}

// Gently taken from: https://github.com/pickfire/parseint
// Implementation details here: https://rust-malaysia.github.io/code/2020/07/11/faster-integer-parsing.html
// And the original source: https://kholdstare.github.io/technical/2020/05/26/faster-integer-parsing.html
pub fn parseint_trick_u64(s: &str) -> u64 {
    let (upper_digits, lower_digits) = s.split_at(8);
    parseint_parse_8_chars(upper_digits) * 100000000 + parseint_parse_8_chars(lower_digits)
}

// Gently taken from: https://github.com/pickfire/parseint
// Implementation details here: https://rust-malaysia.github.io/code/2020/07/11/faster-integer-parsing.html
// And the original source: https://kholdstare.github.io/technical/2020/05/26/faster-integer-parsing.html
fn parseint_parse_8_chars(s: &str) -> u64 {
    let s = s.as_ptr() as *const _;
    let mut chunk = 0;
    unsafe {
        std::ptr::copy_nonoverlapping(s, &mut chunk, std::mem::size_of_val(&chunk));
    }

    // 1-byte mask trick (works on 4 pairs of single digits)
    let lower_digits = (chunk & 0x0f000f000f000f00) >> 8;
    let upper_digits = (chunk & 0x000f000f000f000f) * 10;
    let chunk = lower_digits + upper_digits;

    // 2-byte mask trick (works on 2 pairs of two digits)
    let lower_digits = (chunk & 0x00ff000000ff0000) >> 16;
    let upper_digits = (chunk & 0x000000ff000000ff) * 100;
    let chunk = lower_digits + upper_digits;

    // 4-byte mask trick (works on a pair of four digits)
    let lower_digits = (chunk & 0x0000ffff00000000) >> 32;
    let upper_digits = (chunk & 0x000000000000ffff) * 10000;
    let chunk = lower_digits + upper_digits;

    chunk
}

pub fn parseint_trick_arbitrary_length(s: &str) -> u64 {
    let mut total = 0_u64;
    let mut power = 1_u64;

    // Process the string in 8-character chunks from the end
    for chunk in s.as_bytes().rchunks(8) {
        let chunk_str = unsafe { std::str::from_utf8_unchecked(chunk) };
        let value = if chunk.len() == 8 {
            parseint_parse_8_chars(chunk_str)
        } else {
            // For chunks smaller than 8 characters, use standard parsing
            chunk_str.parse::<u64>().unwrap()
        };
        total += value * power;
        power *= 10_u64.pow(chunk.len() as u32);
    }

    total
}

pub fn parseint_unrolled(s: &str) -> u64 {
    let mut result = 0;
    let bytes = s.as_bytes();
    result += (bytes[0] - b'0') as u64 * 1000000000000000;
    result += (bytes[1] - b'0') as u64 * 100000000000000;
    result += (bytes[2] - b'0') as u64 * 10000000000000;
    result += (bytes[3] - b'0') as u64 * 1000000000000;
    result += (bytes[4] - b'0') as u64 * 100000000000;
    result += (bytes[5] - b'0') as u64 * 10000000000;
    result += (bytes[6] - b'0') as u64 * 1000000000;
    result += (bytes[7] - b'0') as u64 * 100000000;
    result += (bytes[8] - b'0') as u64 * 10000000;
    result += (bytes[9] - b'0') as u64 * 1000000;
    result += (bytes[10] - b'0') as u64 * 100000;
    result += (bytes[11] - b'0') as u64 * 10000;
    result += (bytes[12] - b'0') as u64 * 1000;
    result += (bytes[13] - b'0') as u64 * 100;
    result += (bytes[14] - b'0') as u64 * 10;
    result += (bytes[15] - b'0') as u64;
    result
}

pub fn parseint_unrolled_unsafe(s: &str) -> u64 {
    let mut result = 0;
    let bytes = s.as_bytes();
    result += (unsafe { bytes.get_unchecked(0) } - b'0') as u64 * 1000000000000000;
    result += (unsafe { bytes.get_unchecked(1) } - b'0') as u64 * 100000000000000;
    result += (unsafe { bytes.get_unchecked(2) } - b'0') as u64 * 10000000000000;
    result += (unsafe { bytes.get_unchecked(3) } - b'0') as u64 * 1000000000000;
    result += (unsafe { bytes.get_unchecked(4) } - b'0') as u64 * 100000000000;
    result += (unsafe { bytes.get_unchecked(5) } - b'0') as u64 * 10000000000;
    result += (unsafe { bytes.get_unchecked(6) } - b'0') as u64 * 1000000000;
    result += (unsafe { bytes.get_unchecked(7) } - b'0') as u64 * 100000000;
    result += (unsafe { bytes.get_unchecked(8) } - b'0') as u64 * 10000000;
    result += (unsafe { bytes.get_unchecked(9) } - b'0') as u64 * 1000000;
    result += (unsafe { bytes.get_unchecked(10) } - b'0') as u64 * 100000;
    result += (unsafe { bytes.get_unchecked(11) } - b'0') as u64 * 10000;
    result += (unsafe { bytes.get_unchecked(12) } - b'0') as u64 * 1000;
    result += (unsafe { bytes.get_unchecked(13) } - b'0') as u64 * 100;
    result += (unsafe { bytes.get_unchecked(14) } - b'0') as u64 * 10;
    result += (unsafe { bytes.get_unchecked(15) } - b'0') as u64;
    result
}

pub fn parseint_unrolled_safe(s: &str) -> u64 {
    let mut result = 0;
    let bytes = s.get(..16).unwrap().as_bytes();
    result += (bytes[0] - b'0') as u64 * 1000000000000000;
    result += (bytes[1] - b'0') as u64 * 100000000000000;
    result += (bytes[2] - b'0') as u64 * 10000000000000;
    result += (bytes[3] - b'0') as u64 * 1000000000000;
    result += (bytes[4] - b'0') as u64 * 100000000000;
    result += (bytes[5] - b'0') as u64 * 10000000000;
    result += (bytes[6] - b'0') as u64 * 1000000000;
    result += (bytes[7] - b'0') as u64 * 100000000;
    result += (bytes[8] - b'0') as u64 * 10000000;
    result += (bytes[9] - b'0') as u64 * 1000000;
    result += (bytes[10] - b'0') as u64 * 100000;
    result += (bytes[11] - b'0') as u64 * 10000;
    result += (bytes[12] - b'0') as u64 * 1000;
    result += (bytes[13] - b'0') as u64 * 100;
    result += (bytes[14] - b'0') as u64 * 10;
    result += (bytes[15] - b'0') as u64;
    result
}

pub fn f2s(value: ValueType) -> String {
    ryu::Buffer::new().format_finite(value).to_string()
}

pub fn of2s(value: OrderedValueType) -> String {
    ryu::Buffer::new().format_finite(value.0).to_string()
}

pub fn is_regular_fractal(mode: i8, high: &[ValueType], low: &[ValueType]) -> bool {
    match mode {
        1 => high[4] < high[3] && high[3] < high[2] && high[2] > high[1] && high[1] > high[0],
        -1 => low[4] > low[3] && low[3] > low[2] && low[2] < low[1] && low[1] < low[0],
        _ => false,
    }
}

pub fn is_williams_fractal(mode: i8, high: &[ValueType], low: &[ValueType]) -> bool {
    match mode {
        1 => high[4] < high[2] && high[3] <= high[2] && high[2] >= high[1] && high[2] > high[0],
        -1 => low[4] > low[2] && low[3] >= low[2] && low[2] <= low[1] && low[2] < low[0],
        _ => false,
    }
}

pub fn calc_growth_rate(old: ValueType, new: ValueType) -> ValueType {
    ((new - old) / old) / 100.0
}

pub fn calc_relative_change<T>(old: T, new: T) -> T
where
    T: Float + Into<OrderedFloat<ValueType>> + Copy,
{
    (new - old) / old
}

pub fn calc_change(old: ValueType, new: ValueType) -> (ValueType, ValueType) {
    let change = new - old;
    (change, ((change / old) * 100.0))
}

pub fn calc_ratio_x_to_y(x: ValueType, y: ValueType) -> ValueType {
    (x / y)
}

pub fn calc_ratio(x: ValueType, ratio: ValueType) -> ValueType {
    x * (1.0 + ratio)
}

pub fn adjust_to_percentage(x: ValueType, p: ValueType) -> ValueType {
    (OrderedFloat(x) * (OrderedFloat(1.0) + OrderedFloat(p))).into_inner()
}

pub fn calc_relative_position_between(x: ValueType, low: ValueType, high: ValueType) -> ValueType {
    round_float_to_precision((x - low) / (high - low), 2)
}

pub fn calc_value_from_relative_position(loc: ValueType, low: ValueType, high: ValueType) -> ValueType {
    low + (high - low).abs() * loc
}

pub fn calc_relativity_index(a: ValueType, b: ValueType, c: ValueType) -> ValueType {
    unimplemented!();
}

pub fn calc_percentage_of(x: ValueType, percent: ValueType) -> ValueType {
    x * percent
}

pub fn is_between<F: Float + Into<OrderedFloat<ValueType>>>(x: F, low: F, high: F) -> bool {
    let x = x.into();
    x > low.into() && x < high.into()
}

pub fn is_between_inclusive<F: Float + Into<OrderedFloat<ValueType>>>(x: F, low: F, high: F) -> bool {
    let x = x.into();
    x >= low.into() && x <= high.into()
}

pub fn is_outside<F: Float + Into<OrderedFloat<ValueType>>>(x: F, low: F, high: F) -> bool {
    let x = x.into();
    x < low.into() || x > high.into()
}

pub fn is_outside_inclusive<F: Float + Into<OrderedFloat<ValueType>>>(x: F, low: F, high: F) -> bool {
    let x = x.into();
    x <= low.into() || x >= high.into()
}

pub fn calc_mid_value(l: ValueType, h: ValueType) -> ValueType {
    l + ((h - l) / 2.0)
}

pub fn calc_average_cost(prices_quantity: Vec<(ValueType, ValueType)>) -> Option<ValueType> {
    if prices_quantity.is_empty() {
        return None;
    }

    let mut total_cost = 0.0;
    let mut total_quantity = 0.0;

    for (price, quantity) in prices_quantity {
        if quantity == 0.0 {
            return None;
        }

        total_cost += price * quantity;
        total_quantity += quantity;
    }

    if total_quantity == 0.0 {
        None
    } else {
        Some(total_cost / total_quantity)
    }
}

pub fn round_float_to_precision(f: ValueType, precision: i32) -> ValueType {
    let m = (10.0 as ValueType).powi(precision);
    (f * m).round() / m
}

pub fn round_float_to_ordered_precision(f: ValueType, precision: i32) -> OrderedValueType {
    OrderedFloat(round_float_to_precision(f, precision))
}

pub fn calc_narrowed_proximity(x: ValueType, target: ValueType, boundary: ValueType) -> ValueType {
    match OrderedFloat(x).cmp(&OrderedFloat(target)) {
        Ordering::Less => calc_relative_position_between(x, target, target - boundary).neg(),
        Ordering::Equal => 0.0,
        Ordering::Greater => calc_relative_position_between(x, target, target + boundary),
    }
}

pub fn sort_timeframes_by_duration(timeframes: Vec<Timeframe>) -> Vec<Timeframe> {
    timeframes.into_iter().sorted_by(|a, b| a.duration().cmp(&b.duration())).rev().collect_vec()
}

pub fn coin_specific_timeframe_configs(config: &Config, symbol: Ustr) -> TimeSet<TimeframeConfig> {
    let coin_config = config
        .coins
        .iter()
        .find(|coin_config| coin_config.name == symbol)
        .cloned()
        .expect(format!("coin config for {} not found", symbol).as_str());

    // Coins can specify which timeframes they support.
    // - if no explicit timeframes are specified, then all available timeframes are supported
    // - if explicit timeframes are specified, then only those timeframes are supported
    let timeframes = match coin_config.timeframes {
        None => config
            .default_timeframe_configs
            .iter()
            .sorted_by(|a, b| b.timeframe.duration().cmp(&a.timeframe.duration()))
            .rev()
            .map(|x| x.timeframe)
            .collect_vec(),

        Some(coin_specific_timeframes) => coin_specific_timeframes
            .iter()
            .sorted_by(|&a, &b| b.duration().cmp(&a.duration()))
            .rev()
            .cloned()
            .collect_vec(),
    };

    // Initialize timeframe config for configured timeframes
    let mut timeframe_configs = TimeSet::new();

    // Loading timeframe configs for specified timeframes
    for timeframe_config in config.default_timeframe_configs.iter().filter(|x| timeframes.contains(&x.timeframe)) {
        timeframe_configs.set(timeframe_config.timeframe, timeframe_config.clone());
    }

    timeframe_configs
}

pub fn timeframe_configs_to_timeset(timeframe_configs: Vec<TimeframeConfig>) -> TimeSet<TimeframeConfig> {
    let mut timeset = TimeSet::default();

    for timeframe_config in timeframe_configs {
        timeset.set(timeframe_config.timeframe, timeframe_config);
    }

    timeset
}

pub fn collapse_candles(candles: &[Candle]) -> Candle {
    candles.into_iter().skip(1).fold(candles[0], |acc, c| Candle {
        symbol:                       acc.symbol,
        timeframe:                    acc.timeframe,
        open_time:                    acc.open_time,
        close_time:                   c.close_time,
        open:                         acc.open,
        high:                         acc.high.max(c.high()),
        low:                          acc.low.min(c.low()),
        close:                        c.close(),
        volume:                       acc.volume + c.volume(),
        number_of_trades:             c.number_of_trades,
        quote_asset_volume:           c.quote_asset_volume,
        taker_buy_quote_asset_volume: c.taker_buy_quote_asset_volume,
        taker_buy_base_asset_volume:  c.taker_buy_base_asset_volume,
        is_final:                     c.is_final,
    })
}

pub fn max_fib_leq(input: usize) -> Option<usize> {
    let mut a = 0;
    let mut b = 1;

    while b <= input {
        let temp = a + b;
        a = b;
        b = temp;
    }

    if a == 0 {
        None
    } else {
        Some(a)
    }
}

pub fn timestamp(start: SystemTime) -> u64 {
    let since_epoch = start.duration_since(UNIX_EPOCH).unwrap();
    since_epoch.as_secs() * 1000 + since_epoch.subsec_nanos() as u64 / 1_000_000
}

pub fn max_period(seq: &[usize], input: usize) -> Option<usize> {
    seq.iter().filter(|&&x| x <= input).max().copied()
}

pub fn direction(xs: &[ValueType], last_n: usize, max_faults: usize) -> Direction {
    let start = if last_n > 0 && last_n < xs.len() { xs.len() - last_n } else { 0 };
    let xs = &xs[start..];

    match is_mostly(xs, max_faults) {
        Ordering::Greater => Direction::Rising,
        Ordering::Less => Direction::Falling,
        Ordering::Equal => Direction::Choppy,
    }
}

pub fn is_mostly(xs: &[ValueType], max_faults: usize) -> Ordering {
    let (mut increasing, mut decreasing) = (0usize, 0usize);

    for window in xs.windows(2) {
        match OrderedFloat(window[0]).cmp(&OrderedFloat(window[1])) {
            Ordering::Less => increasing += 1,
            Ordering::Greater => decreasing += 1,
            Ordering::Equal => {}
        }
    }
    if increasing + max_faults >= xs.len() - 1 {
        Ordering::Greater
    } else if decreasing + max_faults >= xs.len() - 1 {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

pub fn motion(xs: &[ValueType], last_n: usize) -> Motion {
    let start = if last_n > 0 && last_n < xs.len() { xs.len() - last_n } else { 0 };
    let xs = &xs[start..];

    match is_accelerating(xs) {
        true => Motion::Accelerating,
        false =>
            if is_decelerating(xs) {
                Motion::Decelerating
            } else {
                Motion::Normal
            },
    }
}

pub fn is_accelerating(xs: &[ValueType]) -> bool {
    let mut deltas = xs.windows(2).map(|window| (window[1] - window[0]).abs());
    let mut last_delta = deltas.next().unwrap_or_default();

    for delta in deltas {
        if delta <= last_delta {
            return false;
        }

        last_delta = delta;
    }

    true
}

pub fn is_decelerating(xs: &[ValueType]) -> bool {
    let mut deltas = xs.windows(2).map(|window| (window[1] - window[0]).abs());
    let mut last_delta = deltas.next().unwrap_or_default();

    for delta in deltas {
        if delta >= last_delta {
            return false;
        }

        last_delta = delta;
    }

    true
}

pub fn all_lt(xs: &[ValueType], y: ValueType) -> bool {
    xs.into_iter().all(|&x| x < y)
}

pub fn all_gt(xs: &[ValueType], y: ValueType) -> bool {
    xs.into_iter().all(|&x| x > y)
}

pub fn calc_nth_exp_growth(base_value: ValueType, nth: usize, r: ValueType) -> ValueType {
    let nth = nth as ValueType;
    base_value * (1.0 + (nth * r)).powf(nth)
}

pub fn calc_nth_exp_decay(base_value: ValueType, nth: usize, r: ValueType) -> ValueType {
    let nth = nth as ValueType;
    base_value * (1.0 - (nth * r)).powf(nth)
}

pub fn calc_proportional_distribution(value: ValueType, dist: QuantityDistribution, n: usize) -> Vec<ValueType> {
    match dist {
        QuantityDistribution::Progressive => {
            let frac: ValueType = value / ((n as ValueType / 2.0) * (1 + n) as ValueType);
            (0..n).map(|x| frac * (x + 1) as ValueType).collect_vec()
        }
        QuantityDistribution::Equal => {
            let frac = value / n as ValueType;
            (0..n).map(|_| frac).collect_vec()
        }
        QuantityDistribution::Degressive => {
            let frac: ValueType = value / ((n as ValueType / 2.0) * (1 + n) as ValueType);
            (0..n).rev().map(|x| frac * (x + 1) as ValueType).collect_vec()
        }
    }
}

pub fn calc_price_distribution(seq: PriceDistribution, lower_limit: ValueType, upper_limit: ValueType, n: usize) -> Vec<ValueType> {
    match seq {
        PriceDistribution::Arithmetic => {
            let x = (upper_limit - lower_limit) / n as ValueType;
            (0..n).map(|i| lower_limit + (x * i as ValueType)).collect()
        }
        PriceDistribution::Geometric => {
            let x = (upper_limit - lower_limit) / n as ValueType;
            let r = (upper_limit / lower_limit).powf(1.0 / n as ValueType);
            (0..n).map(|i| lower_limit + ((x * r) * i as ValueType)).collect()
        }
    }
}

pub fn calc_exponential_distribution(base: ValueType, n: usize, dir: Direction, mut r: ValueType, q: ValueType, step: usize) -> Vec<(ValueType, ValueType)> {
    let mut prev = base;
    (0..(n * step) + 1)
        .step_by(step)
        .map(|i| {
            r *= q;
            match dir {
                Direction::Rising => {
                    let val = calc_nth_exp_growth(base, i, r);
                    let diff_pc = calc_relative_change(prev, val);
                    prev = val;
                    (val, diff_pc)
                }
                Direction::Falling => {
                    let val = calc_nth_exp_decay(base, i, r);
                    let diff_pc = calc_relative_change(prev, val);
                    prev = val;
                    (val, diff_pc)
                }
                _ => panic!("unsupported direction"),
            }
        })
        .skip(1)
        .take(n)
        .collect()
}

/// Applies a provided function `f` to a base value `base`, `n` times iteratively.
/// This function is versatile and can be used for various types of iterative calculations.
///
/// # Arguments
/// * `base` - The initial value to start with.
/// * `n` - The number of iterations to perform.
/// * `f` - A closure that defines the transformation to be applied in each iteration.
///
/// # Returns
/// The result after applying the transformation `n` times.
pub fn apply_function_n_times<F: Copy + FnOnce(ValueType) -> ValueType>(base: ValueType, n: usize, f: F) -> ValueType {
    (0..n).fold(base, |acc, _| f(acc))
}

pub fn calc_slope(now: ValueType, then: ValueType, number_intervals_between: usize) -> ValueType {
    (now - then) / number_intervals_between as ValueType
}

pub fn get_day_f64(x: f64) -> f64 {
    (x / MINS_PER_DAY).floor()
}

pub fn get_hour_f64(x: f64) -> f64 {
    (x.rem_euclid(MINS_PER_DAY) / MINS_PER_HOUR).floor()
}

pub fn get_minute_f64(x: f64) -> f64 {
    x.rem_euclid(MINS_PER_HOUR).floor()
}

pub fn range_overlap(r1: RangeInclusive<ValueType>, r2: RangeInclusive<ValueType>) -> bool {
    r1.start().max(*r2.start()) <= r1.end().min(*r2.end())
}

pub fn swing_highs_and_lows(candles: &[Candle]) -> Vec<(usize, i8, Candle)> {
    candles
        .iter()
        .enumerate()
        .rev()
        .tuple_windows::<(_, _, _, _, _)>()
        .filter_map(|((c4i, c4), (c3i, c3), (c2i, c2), (c1i, c1), (c0i, c0))| {
            let is_swing_low = c4.low >= c2.low && c3.low >= c2.low && c2.low <= c1.low && c2.low <= c0.low;
            let is_swing_high = c4.high <= c2.high && c3.high <= c2.high && c2.high >= c1.high && c2.high >= c0.high;

            if is_swing_low {
                return Some((c2i, -1, *c2));
            } else if is_swing_high {
                return Some((c2i, 1, *c2));
            } else {
                None
            }
        })
        .group_by(|x| x.1 < 0)
        .into_iter()
        // WARNING: deduplicating repeating highs and lows, picking the best corresponding candle
        .filter_map(
            |(is_swing_low, candles)| {
                if is_swing_low {
                    candles.min_by(|a, b| a.2.low.total_cmp(&b.2.low))
                } else {
                    candles.max_by(|a, b| a.2.high.total_cmp(&b.2.high))
                }
            },
        )
        .collect()
}

pub fn detect_swings(data: &[Candle], lookback: usize) -> Vec<(Option<f64>, Option<f64>)> {
    let mut swings = Vec::new();

    for i in 0..data.len() {
        let mut high = Some(data[i].high);
        let mut low = Some(data[i].low);

        for j in i.saturating_sub(lookback)..=i + lookback.min(data.len() - 1) {
            if j != i {
                if data[j].high > data[i].high {
                    high = None;
                }
                if data[j].low < data[i].low {
                    low = None;
                }
            }
        }

        swings.push((high, low));
    }

    swings
}

pub fn fibdown(x: ValueType) -> ValueType {
    x / 1.618
}

pub fn hash_string(symbol: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(symbol.as_bytes());
    hasher.finish()
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 31,
    }
}

pub fn ordinal_suffix(day: u32) -> String {
    match day {
        1 | 21 | 31 => format!("{}st", day),
        2 | 22 => format!("{}nd", day),
        3 | 23 => format!("{}rd", day),
        _ => format!("{}th", day),
    }
}

pub fn format_date_ymd(date: NaiveDate) -> String {
    format!("{} {}, {}", date.format("%B"), ordinal_suffix(date.day()), date.year())
}

pub fn format_date_ym(date: NaiveDate) -> String {
    format!("{} {}", date.format("%B"), date.year())
}

pub fn format_date_ymd_hms(datetime: NaiveDateTime) -> String {
    format!("{} {}, {} {}", datetime.format("%B"), ordinal_suffix(datetime.day()), datetime.year(), datetime.format("%H:%M:%S"))
}

pub fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub fn calculate_pnl(entry_price: OrderedValueType, current_market_price: OrderedValueType, side: NormalizedSide) -> (OrderedValueType, OrderedValueType) {
    match side {
        NormalizedSide::Buy => {
            let delta = current_market_price - entry_price;
            (delta, delta / entry_price)
        }
        NormalizedSide::Sell => {
            let delta = entry_price - current_market_price;
            (delta, delta / entry_price)
        }
    }
}

pub fn data_aspect_from_value(value: ValueType) -> ValueType {
    10000000.0 / 10f64.powf(f64::log10(value).ceil())
}

pub fn float_modulo(a: f64, b: f64) -> f64 {
    a - b * (a / b).floor()
}

#[derive(Debug, Clone)]
pub struct WeightedValue {
    value:      f64,
    weight:     u16,
    is_enabled: bool,
}

pub fn filter_values<'a>(vec: &'a [WeightedValue]) -> impl Iterator<Item = &'a WeightedValue> {
    let len = vec.len();
    vec.iter()
        .enumerate()
        .filter(move |(i, item)| {
            let left = if *i > 0 { vec[i - 1].weight } else { item.weight };
            let right = if *i < len - 1 { vec[i + 1].weight } else { item.weight };
            item.weight >= left && item.weight >= right
        })
        .map(|(_, item)| item)
}

pub fn filter_and_flip_values(vec: &mut [WeightedValue]) {
    let len = vec.len();
    for i in 0..len {
        let left = if i > 0 { vec[i - 1].weight } else { vec[i].weight };
        let right = if i < len - 1 { vec[i + 1].weight } else { vec[i].weight };
        if vec[i].weight < left && vec[i].weight < right {
            vec[i].is_enabled = false;
        }
    }
}

pub fn filter_values_opposite<'a>(vec: &'a [WeightedValue]) -> impl Iterator<Item = &'a WeightedValue> {
    let len = vec.len();
    vec.iter()
        .enumerate()
        .filter(move |(i, item)| {
            let left = if *i > 0 { vec[*i - 1].weight } else { item.weight };
            let right = if *i < len - 1 { vec[*i + 1].weight } else { item.weight };
            let opposite = if left > right {
                if *i < len - 1 {
                    vec[*i + 1].weight
                } else {
                    item.weight
                }
            } else {
                if *i > 0 {
                    vec[*i - 1].weight
                } else {
                    item.weight
                }
            };
            item.weight >= opposite
        })
        .map(|(_, item)| item)
}

pub fn filter_and_flip_values_opposite(vec: &mut [WeightedValue]) {
    let len = vec.len();
    for i in 0..len {
        let left = if i > 0 { vec[i - 1].weight } else { vec[i].weight };
        let right = if i < len - 1 { vec[i + 1].weight } else { vec[i].weight };
        let opposite = if left > right {
            if i < len - 1 {
                vec[i + 1].weight
            } else {
                vec[i].weight
            }
        } else {
            if i > 0 {
                vec[i - 1].weight
            } else {
                vec[i].weight
            }
        };
        if vec[i].weight < opposite {
            vec[i].is_enabled = false;
        }
    }
}

pub fn approx_eq(a: ValueType, b: ValueType, epsilon: ValueType) -> bool {
    (a - b).abs() < epsilon
}

#[inline]
pub fn truncate_to_midnight(time: DateTime<Utc>) -> DateTime<Utc> {
    let time = time.naive_utc();
    // NaiveDateTime::new(time.date(), time.time().with_hour().with_second(0).unwrap().with_nanosecond(0).unwrap()).and_utc()
    // truncate to midnight
    NaiveDateTime::new(time.date(), NaiveTime::from_hms_opt(0, 0, 0).unwrap()).and_utc()
}

#[inline]
pub fn remove_seconds(time: DateTime<Utc>) -> DateTime<Utc> {
    let time = time.naive_utc();
    NaiveDateTime::new(time.date(), time.time().with_second(0).unwrap().with_nanosecond(0).unwrap()).and_utc()
}

#[inline]
pub fn remove_milliseconds(time: DateTime<Utc>) -> DateTime<Utc> {
    let time = time.naive_utc();
    NaiveDateTime::new(time.date(), time.time().with_nanosecond(0).unwrap()).and_utc()
}

#[inline]
pub fn datetime_round_down_to_base_ms(datetime: DateTime<Utc>, base_ms: i64) -> DateTime<Utc> {
    let total_ms = datetime.timestamp_millis(); // Get total milliseconds of the datetime
    let base_rounded_ms = (total_ms / base_ms) * base_ms; // Calculate the nearest lower multiple of base_ms
    let rounded_datetime = DateTime::<Utc>::from_naive_utc_and_offset(
        NaiveDateTime::from_timestamp_opt(
            base_rounded_ms / 1000,
            ((base_rounded_ms % 1000) * 1_000_000) as u32, // Convert remaining milliseconds to nanoseconds
        )
        .unwrap(),
        Utc,
    );

    rounded_datetime
}

pub fn parse_candle_data_array(symbol: Ustr, timeframe: Timeframe, text: &str) -> Vec<Candle> {
    let data: Vec<Vec<serde_json::Value>> = serde_json::from_str(text).expect("JSON was not well-formatted");
    let current_time = Utc::now();

    data.into_iter()
        .map(|arr| {
            let open_time = Utc.timestamp_millis_opt(arr[0].as_i64().unwrap()).unwrap();
            let close_time = Utc.timestamp_millis_opt(arr[6].as_i64().unwrap()).unwrap();

            // Assuming a candle is final if its close time is in the past relative to the current time
            let is_final = (Utc::now() - open_time) >= timeframe.duration();

            Candle {
                symbol:                       symbol,
                timeframe:                    timeframe,
                open_time:                    open_time,
                open:                         s2f(arr[1].as_str().unwrap()),
                high:                         s2f(arr[2].as_str().unwrap()),
                low:                          s2f(arr[3].as_str().unwrap()),
                close:                        s2f(arr[4].as_str().unwrap()),
                volume:                       s2f(arr[5].as_str().unwrap()),
                close_time:                   close_time,
                quote_asset_volume:           s2f(arr[7].as_str().unwrap()),
                number_of_trades:             arr[8].as_i64().unwrap(),
                taker_buy_base_asset_volume:  s2f(arr[9].as_str().unwrap()),
                taker_buy_quote_asset_volume: s2f(arr[10].as_str().unwrap()),
                is_final:                     is_final,
            }
        })
        .collect()
}

pub fn dump_to_file(filename: &str, data: &str, append: bool) -> Result<()> {
    use std::{
        fs::OpenOptions,
        io::{prelude::*, Result},
    };

    let mut file = OpenOptions::new().write(true).create(true).append(append).open(filename)?;

    writeln!(file, "{}", data)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::BufReader, path::Path, time::Instant};

    use yata::{
        core::{Method, MovingAverageConstructor},
        helpers::MA,
        methods::SMA,
    };

    use crate::{
        candle_manager::CandleBackupDate,
        countdown::Countdown,
        simulator_generator::{generate_candles, generate_price_sequence, GeneratorConfig},
        util::calc_value_from_relative_position,
    };

    use super::*;

    #[test]
    fn test_datetime_rounding() {
        let now = Utc::now();
        println!("Current time: {}", now);

        let rounded_100ms = datetime_round_down_to_base_ms(now, 100);
        println!("Rounded to 100ms: {}", rounded_100ms);

        let rounded_250ms = datetime_round_down_to_base_ms(now, 250);
        println!("Rounded to 250ms: {}", rounded_250ms);

        let rounded_500ms = datetime_round_down_to_base_ms(now, 500);
        println!("Rounded to 500ms: {}", rounded_500ms);
    }

    #[test]
    fn test_value_filtering() {
        let mut values = vec![
            WeightedValue {
                value:      1.2,
                weight:     1,
                is_enabled: true,
            },
            WeightedValue {
                value:      2.3,
                weight:     2,
                is_enabled: true,
            },
            WeightedValue {
                value:      3.4,
                weight:     1,
                is_enabled: true,
            },
            WeightedValue {
                value:      4.5,
                weight:     2,
                is_enabled: true,
            },
            WeightedValue {
                value:      1.5,
                weight:     3,
                is_enabled: true,
            },
            WeightedValue {
                value:      2.7,
                weight:     4,
                is_enabled: true,
            },
            WeightedValue {
                value:      3.1,
                weight:     2,
                is_enabled: true,
            },
            WeightedValue {
                value:      4.3,
                weight:     5,
                is_enabled: true,
            },
            WeightedValue {
                value:      1.8,
                weight:     4,
                is_enabled: true,
            },
            WeightedValue {
                value:      2.4,
                weight:     3,
                is_enabled: true,
            },
            WeightedValue {
                value:      3.2,
                weight:     2,
                is_enabled: true,
            },
            WeightedValue {
                value:      4.6,
                weight:     6,
                is_enabled: true,
            },
            WeightedValue {
                value:      1.1,
                weight:     5,
                is_enabled: true,
            },
            WeightedValue {
                value:      2.8,
                weight:     6,
                is_enabled: true,
            },
            WeightedValue {
                value:      3.5,
                weight:     4,
                is_enabled: true,
            },
            WeightedValue {
                value:      4.7,
                weight:     7,
                is_enabled: true,
            },
        ];

        // filter_and_flip_values(&mut values);
        filter_and_flip_values_opposite(&mut values);

        for v in values {
            println!("{:02} {:02} {}", v.value, v.weight, v.is_enabled);
        }

        /*
        let filtered = filter_values(&values).collect::<Vec<_>>();

        for i in &filtered {
            println!("Value: {}, Weight: {}", i.value, i.weight);
        }
        */
    }

    #[test]
    fn test_float_modulo() {
        println!("{}", float_modulo(100.0, 0.23));
    }

    #[test]
    fn test_overlaps() {
        let r1 = 0.0 as ValueType..=100.0;
        let r2 = 101.0 as ValueType..=200.0;
        let r3 = 90.0 as ValueType..=200.0;

        assert_eq!(range_overlap(r1.clone(), r2), false);
        assert_eq!(range_overlap(r1, r3), true);
    }

    #[test]
    fn test_swing_highs_and_lows() {
        let candles = generate_candles(GeneratorConfig::default(), Timeframe::M1, Utc::now());
        println!("{:#?}", swing_highs_and_lows(&candles));
    }

    #[test]
    fn test_relativity() {
        let low = 20.0;
        let high = 80.0;

        println!("{:?}", calc_relative_position_between(92.92, low, high));
        println!("{:?}", calc_relative_position_between(62.92, low, high));
        println!("{:?}", calc_relative_position_between(12.92, low, high));
    }

    #[test]
    fn test_weight_distributions() {
        println!("{:#?}", calc_proportional_distribution(1000.0, QuantityDistribution::Progressive, 5));
        println!("{:#?}", calc_proportional_distribution(1000.0, QuantityDistribution::Equal, 5));
        println!("{:#?}", calc_proportional_distribution(1000.0, QuantityDistribution::Degressive, 5));
    }

    #[test]
    fn test_price_distributions() {
        let lower_limit = 34.32;
        let upper_limit = 48.60;
        let n = 12;
        let delta = upper_limit - lower_limit;

        eprintln!("lower_limit = {:#?}", lower_limit);
        eprintln!("upper_limit = {:#?}", upper_limit);
        eprintln!("n = {:#?}", n);
        eprintln!("delta = {:#?}", delta);
        eprintln!("arithmetic = {:#?}", calc_price_distribution(PriceDistribution::Arithmetic, lower_limit, upper_limit, n));
        eprintln!("geometric = {:#?}", calc_price_distribution(PriceDistribution::Geometric, lower_limit, upper_limit, n));
    }

    #[test]
    fn test_shotgun_distribution() {
        let lower_limit = 16823.06;
        let upper_limit = 16825.06;
        let n = 3;
        let delta = upper_limit - lower_limit;
        let quantity = 0.00856;

        let dist_arithmetic_deg = calc_price_distribution(PriceDistribution::Geometric, lower_limit, upper_limit, n)
            .into_iter()
            .zip(calc_proportional_distribution(quantity, QuantityDistribution::Degressive, 5))
            .collect_vec();

        let dist_geometric_deg = calc_price_distribution(PriceDistribution::Geometric, lower_limit, upper_limit, n)
            .into_iter()
            .zip(calc_proportional_distribution(quantity, QuantityDistribution::Degressive, 5))
            .collect_vec();

        let dist_arithmetic_prog = calc_price_distribution(PriceDistribution::Geometric, lower_limit, upper_limit, n)
            .into_iter()
            .zip(calc_proportional_distribution(quantity, QuantityDistribution::Degressive, 5))
            .collect_vec();

        let dist_geometric_prog = calc_price_distribution(PriceDistribution::Geometric, lower_limit, upper_limit, n)
            .into_iter()
            .zip(calc_proportional_distribution(quantity, QuantityDistribution::Degressive, 5))
            .collect_vec();

        eprintln!("lower_limit = {:?}", lower_limit);
        eprintln!("upper_limit = {:?}", upper_limit);
        eprintln!("n = {:#?}", n);
        eprintln!("delta = {:#?}", delta);
        eprintln!("arithmetic degressive = {:?}", dist_arithmetic_deg);
        eprintln!("geometric degressive = {:?}", dist_geometric_deg);
        eprintln!("arithmetic progressive = {:?}", dist_arithmetic_prog);
        eprintln!("geometric progressive = {:?}", dist_geometric_prog);
    }

    #[test]
    fn test_exponential_distribution() {
        eprintln!("desc = {:#?}", calc_exponential_distribution(16795.82, 10, Direction::Falling, 0.01, 1.0, 1));
        eprintln!("asc = {:#?}", calc_exponential_distribution(16795.82, 10, Direction::Rising, 0.01, 1.0, 1));
    }

    #[test]
    fn test_order_checking() {
        assert_eq!(all_gt(&[1.0, 2.0, 3.0], 0.0), true);
        assert_eq!(all_gt(&[1.0, -1.0, 3.0], 0.0), false);
        assert_eq!(all_lt(&[1.0, 2.0, 3.0], 3.01), true);
        assert_eq!(all_lt(&[-1.0, 5.0, 3.0], 0.0), false);
        /*
        assert_eq!(all_rising_in_order(&[1.0, 2.0, 3.0]), true);
        assert_eq!(all_rising_in_order(&[1.0, 2.0, 1.82, 3.0]), false);
        assert_eq!(all_falling_in_order(&[3.0, 2.0, 1.0]), true);
        assert_eq!(all_falling_in_order(&[3.0, 1.82, 2.0, 3.0]), false);
        assert_eq!(is_mostly_rising(&[1.0, 2.0, 1.82, 3.0], 1), true);
        assert_eq!(is_mostly_rising(&[1.0, 1.0, 1.82, 3.0], 1), true);
        assert_eq!(is_mostly_rising(&[1.0, 1.0, 1.0, 1.82, 3.0], 1), false);
        assert_eq!(is_mostly_rising(&[1.0, 1.2, 1.1, 0.9, 3.0], 1), false);
        assert_eq!(is_mostly_falling(&[3.0, 2.0, 1.82, 1.0], 1), true);
        assert_eq!(is_mostly_falling(&[3.0, 2.0, 1.0, 1.0], 1), true);
        assert_eq!(is_mostly_falling(&[3.0, 2.0, 2.0, 1.82, 3.82], 1), false);
        assert_eq!(is_mostly_falling(&[3.0, 1.2, 1.3, 0.9, 1.0], 1), false);
         */
    }

    #[test]
    fn test_value_from_location() {
        println!("{:#?}", calc_value_from_relative_position(0.25, 0.0, 100.));
        println!("{:#?}", calc_value_from_relative_position(0.75, 0.0, 100.));
    }

    #[test]
    fn test_extract_coinlist() {
        let datafile = File::open(Path::new("data/exchange_info.json")).unwrap();
        let exinfo: ExchangeInformation = serde_json::from_reader(BufReader::new(datafile)).unwrap();
        let symbols: Vec<Ustr> = exinfo
            .symbols
            .into_iter()
            .filter_map(|s| if s.symbol.ends_with("USDT") { Some(s.symbol) } else { None })
            .collect();

        println!("{:#?}", symbols.len());
        println!("{:#?}", symbols.into_iter().for_each(|x| println!("{}", x)));
    }

    #[test]
    fn test_large_series_window() {
        let ticks = generate_price_sequence(17000.0, 1000000, 0.0005);

        println!("{:#?}", ticks.len());

        let mut r0 = SMA::new(500, &0.0).unwrap();
        let mut r1 = SMA::new(309, &0.0).unwrap();

        let mut g0 = SMA::new(800, &0.0).unwrap();
        let mut g1 = SMA::new(496, &0.0).unwrap();

        let mut b0 = SMA::new(1300, &0.0).unwrap();
        let mut b1 = SMA::new(807, &0.0).unwrap();

        // warmup
        let t0 = ticks.iter().take(5000).cloned().collect_vec();

        let it = Instant::now();
        for t in t0 {
            r1.next(&r0.next(&t));
            g1.next(&g0.next(&t));
            b1.next(&b0.next(&t));
        }
        println!("{:#?}", it.elapsed());

        let t1 = ticks.iter().take(5000).cloned().collect_vec();

        let it = Instant::now();
        for t in t1 {
            r1.next(&r0.next(&t));
            g1.next(&g0.next(&t));
            b1.next(&b0.next(&t));
        }
        println!("{:#?}", it.elapsed());

        // rest
        let it = Instant::now();
        for (i, t) in ticks.iter().enumerate().take(1000) {
            r1.next(&r0.next(&t));
            g1.next(&g0.next(&t));
            b1.next(&b0.next(&t));
        }
        println!("{:#?}", it.elapsed());
    }

    #[test]
    fn test_large_series_window_2() {
        let mut srs = [1.5, 2.1, 1.3, 5.5, 1.1, 0.82].map(|x| OrderedFloat(x));
        println!("{:#?}", srs);
        srs.sort_by(|a, b| a.cmp(&b));
        println!("{:#?}", srs);
        // println!("{:#?}", srs.iter().rev().collect_vec());

        let price = 16566.83;
        let p0001 = 16566.83 * 0.0001;
        eprintln!("calc_x_is_percentage_of_y(x, price) = {:?}", calc_ratio_x_to_y(p0001, price));
        eprintln!("calc_x_is_percentage_of_y(0.40, p0001) = {:?}", calc_ratio_x_to_y(0.45, p0001));

        let ticks = generate_price_sequence(17000.0, 1000, 0.0005);

        let fibseq = vec![
            3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946, 17711, 28657, 46368, 75025, 121393, 196418, 317811,
        ];

        let it = Instant::now();
        let mut emas = fibseq.into_iter().map(|x| (x, MA::EMA(x).init(0.0).unwrap())).collect_vec();
        println!("{:#?}", it.elapsed());

        let it = Instant::now();
        for x in ticks {
            emas.iter_mut().for_each(|(size, ema)| {
                ema.next(&x);
            });
        }
        println!("{:#?}", it.elapsed());

        dbg!(emas);
    }

    #[test]
    fn test_acceleration_and_deceleration() {
        // acceleration
        assert!(is_accelerating(&[0.1, 0.2, 0.5, 1.2, 2.0, 5.0]));
        assert_eq!(motion(&[0.1, 0.2, 0.5, 1.2, 2.0, 5.0], 0), Motion::Accelerating);

        assert!(!is_accelerating(&[0.1, 0.3, 0.5, 0.8, 1.1, 3.0]));
        assert_eq!(motion(&[0.1, 0.3, 0.5, 0.8, 1.1, 3.0], 0), Motion::Normal);

        assert!(is_accelerating(&[3.0, 2.8, 2.5, 2.1, 1.5, 0.1]));
        assert_eq!(motion(&[3.0, 2.8, 2.5, 2.1, 1.5, 0.1], 0), Motion::Accelerating);

        assert!(!is_accelerating(&[3.0, 2.8, 2.5, 2.2, 1.5, 0.5]));
        assert_eq!(motion(&[3.0, 2.8, 2.5, 2.2, 1.5, 0.5], 0), Motion::Normal);

        // deceleration
        assert!(is_decelerating(&[1.0, 2.0, 2.8, 3.5, 4.1, 4.2]));
        assert_eq!(motion(&[1.0, 2.0, 2.8, 3.5, 4.1, 4.2], 0), Motion::Decelerating);

        assert!(!is_decelerating(&[1.0, 2.0, 2.8, 3.9, 4.1, 4.2]));
        assert_eq!(motion(&[1.0, 2.0, 2.8, 3.9, 4.1, 4.2], 0), Motion::Normal);

        assert!(is_decelerating(&[5.0, 4.0, 3.2, 2.5, 2.1, 1.8]));
        assert_eq!(motion(&[5.0, 4.0, 3.2, 2.5, 2.1, 1.8], 0), Motion::Decelerating);

        assert!(!is_decelerating(&[5.0, 4.0, 3.2, 2.8, 2.1, 1.8]));
        assert_eq!(motion(&[5.0, 4.0, 3.2, 2.8, 2.1, 1.8], 0), Motion::Normal);
    }

    #[test]
    fn test_formatted_dates() {
        (0..=121).for_each(|x| {
            println!("{}", CandleBackupDate::month_from_date((Utc::now() - chrono::Duration::days(x as i64)).date_naive()).to_formatted_string());
        });

        (0..=365).for_each(|x| {
            println!("{}", CandleBackupDate::day_from_date((Utc::now() - chrono::Duration::days(x as i64)).date_naive()).to_formatted_string());
        });
    }

    #[test]
    fn test_find_max_leq() {
        let numbers = [
            3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946, 17711, 28657, 46368, 75025, 121393, 196418, 317811, 514229,
            832040, 1346269,
        ];

        // Test function with each number in the list
        for i in 3..numbers.len() {
            assert_eq!(max_period(numbers.as_slice(), numbers[i] + 1), Some(numbers[i]));
        }

        // Test function with numbers in between the list
        assert_eq!(max_period(numbers.as_slice(), 15), Some(13));
        assert_eq!(max_period(numbers.as_slice(), 23), Some(21));
        assert_eq!(max_period(numbers.as_slice(), 52), Some(34));

        // Test function with lower and upper edge cases
        assert_eq!(max_period(numbers.as_slice(), 0), None);
        assert_eq!(max_period(numbers.as_slice(), 4), Some(3));
        assert_eq!(max_period(numbers.as_slice(), 7), Some(5));
        assert_eq!(max_period(numbers.as_slice(), 1346270), Some(1346269));

        // Test function with numbers larger than the list
        assert_eq!(max_period(numbers.as_slice(), 5000000), Some(1346269));
    }

    #[test]
    fn test_is_accelerating() {
        let data = vec![1.0, 2.0, 4.0, 8.0, 16.0];
        assert_eq!(is_accelerating(&data), true);

        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(is_accelerating(&data), false);
    }

    #[test]
    fn test_is_decelerating() {
        let data = vec![16.0, 8.0, 4.0, 2.0, 1.0];
        assert_eq!(is_decelerating(&data), true);

        let data = vec![5.0, 4.0, 3.0, 2.0, 1.0];
        assert_eq!(is_decelerating(&data), false);
    }

    #[test]
    fn test_is_mostly() {
        let data = vec![1.0, 2.0, 3.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(is_mostly(&data, 1), Ordering::Greater);

        let data = vec![5.0, 4.0, 3.0, 4.0, 3.0, 2.0, 1.0];
        assert_eq!(is_mostly(&data, 1), Ordering::Less);

        let data = vec![1.0, 2.0, 1.0, 2.0, 1.0, 2.0, 1.0];
        assert_eq!(is_mostly(&data, 0), Ordering::Equal);
    }

    #[test]
    fn test_data_aspect() {
        println!("{}", 10000000.0 / (10f64.powf(f64::log10(30303.03).ceil())));
        println!("{}", 10000000.0 / (10f64.powf(f64::log10(1.10823).ceil())));
    }

    #[test]
    fn test_parse_int() {
        // Testing the original function with a 16-digit number
        assert_eq!(parseint_trick_u64("1234567890123456"), 1234567890123456_u64);

        // Testing the new function with various lengths
        assert_eq!(parseint_trick_arbitrary_length("1"), 1);
        assert_eq!(parseint_trick_arbitrary_length("12"), 12);
        assert_eq!(parseint_trick_arbitrary_length("12345678"), 12345678);
        assert_eq!(parseint_trick_arbitrary_length("123456789"), 123456789);
        assert_eq!(parseint_trick_arbitrary_length("1234567890123456"), 1234567890123456);
        assert_eq!(parseint_trick_arbitrary_length("12345678901234567"), 12345678901234567);
    }

    #[test]
    fn test_countdown() {
        let mut countdown = Countdown::new(4u8);

        assert!(countdown.checkin());
        assert!(countdown.checkin());
        assert!(countdown.checkin());
        assert!(countdown.checkin());
        assert!(!countdown.checkin());
    }
}
