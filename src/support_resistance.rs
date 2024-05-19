use std::{
    borrow::Borrow,
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    ops::Neg,
};

use itertools::{
    FoldWhile::{Continue, Done},
    Itertools,
};
use num_traits::{Float, Zero};
use ordered_float::OrderedFloat;
use ustr::{ustr, Ustr};
use yata::core::{PeriodType, ValueType};

use crate::{
    candle::Candle,
    types::OrderedValueType,
    util::{calc_mid_value, calc_narrowed_proximity, calc_relative_position_between, is_between, is_between_inclusive},
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default, Ord, PartialOrd)]
pub enum MaType {
    #[default]
    EMA,
    WMA,
    WWMA,
    WSMA,
    RMA,
    SMA,
}

impl AsRef<str> for MaType {
    fn as_ref(&self) -> &str {
        match self {
            MaType::EMA => "EMA",
            MaType::WMA => "WMA",
            MaType::WWMA => "WWMA",
            MaType::WSMA => "WSMA",
            MaType::RMA => "RMA",
            MaType::SMA => "SMA",
        }
    }
}

impl Display for MaType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub struct SRLevel {
    pub r#type: MaType,
    pub period: PeriodType,
    pub name:   Ustr,
    pub value:  OrderedValueType,
}

impl Ord for SRLevel {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl PartialEq for SRLevel {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for SRLevel {}

impl PartialOrd for SRLevel {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl SRLevel {
    pub fn new(r#type: MaType, period: PeriodType, value: OrderedValueType) -> Self {
        Self {
            r#type,
            period,
            value,
            name: ustr(format!("{}_{}", r#type.as_ref(), period).as_str()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_zero()
    }
}

macro_rules! impl_pivot_common {
    () => {
        pub fn index_to_price(&self, loc: ValueType) -> Option<ValueType> {
            let (lvl, loc) = (loc.trunc() as i8, loc.fract());
            let mut max_lvl = Self::LEVELS as i8;

            if lvl == max_lvl {
                return self.level2(max_lvl);
            } else if lvl == -max_lvl {
                return self.level2(-max_lvl);
            }

            match lvl.is_positive() {
                true => match (self.level2(lvl), self.level2(lvl + 1)) {
                    (Some(p), Some(next_p)) => Some(p + (next_p - p).abs() * loc),
                    _ => None,
                },
                false => match (self.level2(lvl), self.level2(lvl - 1)) {
                    (Some(p), Some(prev_p)) => Some(p + (prev_p - p).abs() * loc),
                    _ => None,
                },
            }
        }

        pub fn price_to_index(&self, x: ValueType) -> Option<ValueType> {
            match OrderedFloat(x).cmp(&OrderedFloat(self.pp())) {
                Ordering::Less => match self.arr[0..=Self::LEVELS]
                    .iter()
                    .rev()
                    .find_position(|&&p| OrderedFloat(x).ge(&OrderedFloat(p)))
                {
                    None => None,
                    Some((i, &p)) => match self.level2(-(i as i8) + 1) {
                        None => None,
                        Some(prev) => Some(-(i as ValueType - calc_relative_position_between(x, p, prev))),
                    },
                },
                Ordering::Equal => {
                    return Some(0.0);
                }
                Ordering::Greater => match self.arr[Self::PP_INDEX..]
                    .iter()
                    .find_position(|&&p| OrderedFloat(x).le(&OrderedFloat(p)))
                {
                    None => None,
                    Some((i, &p)) => match self.level2((i as i8) - 1) {
                        None => None,
                        Some(next) => Some((i as ValueType - calc_relative_position_between(x, p, next))),
                    },
                },
            }
        }

        pub fn pp(&self) -> ValueType {
            self.arr[Self::PP_INDEX]
        }

        fn is_level_valid(&self, lvl: i8) -> bool {
            lvl.saturating_abs() <= Self::LEVELS as i8
        }

        pub fn level2(&self, lvl: i8) -> Option<ValueType> {
            match self.is_level_valid(lvl) {
                false => None,
                true => Some(self.arr[(Self::PP_INDEX as i8 + lvl) as usize]),
            }
        }

        pub fn level(&self, lvl: i8) -> Option<ValueType> {
            let lvl = match lvl.cmp(&0) {
                Ordering::Less => lvl - 1,
                Ordering::Equal => 0,
                Ordering::Greater => lvl + 1,
            };

            match self.is_level_valid(lvl) {
                false => None,
                true => Some(self.arr[(Self::PP_INDEX as i8 + lvl) as usize]),
            }
        }
    };
}

#[derive(Debug)]
pub enum SR {
    Classic,
    Woodie,
    Camarilla,
    Fibonacci,
    DeMark,
}

#[derive(Debug, Copy, Clone)]
pub struct Classic {
    arr:   [ValueType; 17],
    pub p: ValueType,
    pub r: [ValueType; 4],
    pub s: [ValueType; 4],
}

impl Classic {
    const LEVELS: usize = 8;
    const PP_INDEX: usize = 8;

    pub fn new(Candle { open, high, low, close, .. }: &Candle) -> Self {
        let pp = (high + low + close) / 3.0;
        let range = high - low;

        let r1 = (pp * 2.0) - low;
        let r2 = pp + range;
        let r3 = pp + (2.0 * range);
        let r4 = pp + (3.0 * range);

        let rm1 = calc_mid_value(pp, r1);
        let rm2 = calc_mid_value(r1, r2);
        let rm3 = calc_mid_value(r2, r3);
        let rm4 = calc_mid_value(r3, r4);

        let s1 = (2.0 * pp) - high;
        let s2 = pp - range;
        let s3 = pp - (2.0 * range);
        let s4 = pp - (3.0 * range);

        let sm1 = calc_mid_value(s1, pp);
        let sm2 = calc_mid_value(s2, s1);
        let sm3 = calc_mid_value(s3, s2);
        let sm4 = calc_mid_value(s4, s3);

        Self {
            arr: [s4, sm4, s3, sm3, s2, sm2, s1, sm1, pp, rm1, r1, rm2, r2, rm3, r3, rm4, r4],
            p:   pp,
            r:   [r1, r2, r3, r4],
            s:   [s1, s2, s3, s4],
        }
    }

    impl_pivot_common!();
}

#[derive(Debug, Copy, Clone)]
pub struct Woodie {
    arr:   [ValueType; 17],
    pub p: ValueType,
    pub r: [ValueType; 4],
    pub s: [ValueType; 4],
}

impl Woodie {
    const LEVELS: usize = 8;
    const PP_INDEX: usize = 8;

    pub fn new(Candle { open, high, low, close, .. }: &Candle) -> Self {
        let pp = (high + low + (2.0 * open)) / 4.0;
        let range = high - low;

        let r1 = (pp * 2.0) - low;
        let r2 = pp + range;
        let r3 = high + (2.0 * (pp - low));
        let r4 = pp + (3.0 * range);

        let rm1 = calc_mid_value(pp, r1);
        let rm2 = calc_mid_value(r1, r2);
        let rm3 = calc_mid_value(r2, r3);
        let rm4 = calc_mid_value(r3, r4);

        let s1 = (2.0 * pp) - high;
        let s2 = pp - range;
        let s3 = low - (2.0 * (high - pp));
        let s4 = pp - (3.0 * range);

        let sm1 = calc_mid_value(s1, pp);
        let sm2 = calc_mid_value(s2, s1);
        let sm3 = calc_mid_value(s3, s2);
        let sm4 = calc_mid_value(s4, s3);

        Self {
            arr: [s4, sm4, s3, sm3, s2, sm2, s1, sm1, pp, rm1, r1, rm2, r2, rm3, r3, rm4, r4],
            p:   pp,
            r:   [r1, r2, r3, r4],
            s:   [s1, s2, s3, s4],
        }
    }

    impl_pivot_common!();
}

#[derive(Debug, Copy, Clone)]
pub struct Camarilla {
    arr:   [ValueType; 25],
    pub p: ValueType,
    pub r: [ValueType; 6],
    pub s: [ValueType; 6],
}

impl Camarilla {
    const LEVELS: usize = 12;
    const PP_INDEX: usize = 12;

    pub fn new(Candle { open, high, low, close, .. }: &Candle) -> Self {
        let pp = (high + low + close) / 3.0;
        let range = high - low;

        let r1 = close + (0.091666667 * range);
        let r2 = close + (0.183333333 * range);
        let r3 = close + (0.275 * range);
        let r4 = close + (0.55 * range);
        let r5 = r4 + (1.168 * (r4 - r3));
        let r6 = (high / low) * close;

        let rm1 = calc_mid_value(pp, r1);
        let rm2 = calc_mid_value(r1, r2);
        let rm3 = calc_mid_value(r2, r3);
        let rm4 = calc_mid_value(r3, r4);
        let rm5 = calc_mid_value(r4, r5);
        let rm6 = calc_mid_value(r5, r6);

        let s1 = close - (0.091666667 * range);
        let s2 = close - (0.183333333 * range);
        let s3 = close - (0.275 * range);
        let s4 = close - (0.55 * range);
        let s5 = s4 - (1.168 * (s3 - s4));
        let s6 = close - (r6 - close);

        let sm1 = calc_mid_value(s1, pp);
        let sm2 = calc_mid_value(s2, s1);
        let sm3 = calc_mid_value(s3, s2);
        let sm4 = calc_mid_value(s4, s3);
        let sm5 = calc_mid_value(s5, s4);
        let sm6 = calc_mid_value(s6, s5);
        Self {
            arr: [
                s6, sm6, s5, sm5, s4, sm4, s3, sm3, s2, sm2, s1, sm1, pp, rm1, r1, rm2, r2, rm3, r3, rm4, r4, rm5, r5, rm6, r6,
            ],
            p:   pp,
            s:   [s1, s2, s3, s4, s5, s6],
            r:   [r1, r2, r3, r4, r5, r6],
        }
    }

    impl_pivot_common!();
}

#[derive(Debug, Copy, Clone)]
pub struct Fibonacci {
    arr:   [ValueType; 29],
    pub p: ValueType,
    pub r: [ValueType; 7],
    pub s: [ValueType; 7],
}

impl Fibonacci {
    const LEVELS: usize = 12;
    const PP_INDEX: usize = 15;

    pub fn new(Candle { open, high, low, close, .. }: &Candle) -> Self {
        let pp = (high + low + close) / 3.0;
        let range = high - low;

        let r1 = close + (0.382 * range);
        let r2 = close + (0.50 * range);
        let r3 = close + (0.618 * range);
        let r4 = close + range;
        let r5 = close + (1.382 * range);
        let r6 = close + (2.000 * range);
        let r7 = close + (2.618 * range);

        let rm1 = calc_mid_value(pp, r1);
        let rm2 = calc_mid_value(r1, r2);
        let rm3 = calc_mid_value(r2, r3);
        let rm4 = calc_mid_value(r3, r4);
        let rm5 = calc_mid_value(r4, r5);
        let rm6 = calc_mid_value(r5, r6);
        let rm7 = calc_mid_value(r6, r7);

        let s1 = close - (0.382 * range);
        let s2 = close - (0.50 * range);
        let s3 = close - (0.618 * range);
        let s4 = close - range;
        let s5 = close - (1.382 * range);
        let s6 = close - (2.000 * range);
        let s7 = close - (2.618 * range);

        let sm1 = calc_mid_value(s1, pp);
        let sm2 = calc_mid_value(s2, s1);
        let sm3 = calc_mid_value(s3, s2);
        let sm4 = calc_mid_value(s4, s3);
        let sm5 = calc_mid_value(s5, s4);
        let sm6 = calc_mid_value(s6, s5);
        let sm7 = calc_mid_value(s7, s6);

        Self {
            arr: [
                s7, sm7, s6, sm6, s5, sm5, s4, sm4, s3, sm3, s2, sm2, s1, sm1, pp, rm1, r1, rm2, r2, rm3, r3, rm4, r4, rm5, r5, rm6, r6, rm7, r7,
            ],
            p:   pp,
            s:   [s1, s2, s3, s4, s5, s6, s7],
            r:   [r1, r2, r3, r4, r5, r6, r7],
        }
    }

    impl_pivot_common!();
}

#[derive(Debug, Copy, Clone)]
pub struct DeMark {
    arr:   [ValueType; 5],
    pub p: ValueType,
    pub r: [ValueType; 1],
    pub s: [ValueType; 1],
}

impl DeMark {
    const LEVELS: usize = 2;
    const PP_INDEX: usize = 2;

    pub fn new(Candle { open, high, low, close, .. }: &Candle) -> Self {
        let x = match close.partial_cmp(open).unwrap() {
            Ordering::Less => high + (2.0 * low) + close,
            Ordering::Equal => high + low + (2.0 * close),
            Ordering::Greater => (2.0 * high) + low + close,
        };

        let pp = x / 4.0;

        let r1 = (x / 2.0) - low;
        let rm1 = calc_mid_value(pp, r1);

        let s1 = (x / 2.0) - high;
        let sm1 = calc_mid_value(s1, pp);

        Self {
            arr: [s1, sm1, pp, rm1, r1],
            p:   pp,
            r:   [r1],
            s:   [s1],
        }
    }

    impl_pivot_common!();
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU32,
        thread::sleep,
        time::{Duration, Instant},
    };

    use num_traits::ToPrimitive;

    use crate::util::{calc_narrowed_proximity, round_float_to_precision};

    use super::*;

    #[test]
    fn test_points() {
        let c = Candle {
            symbol:                       Default::default(),
            timeframe:                    Default::default(),
            open_time:                    Default::default(),
            close_time:                   Default::default(),
            open:                         20010.0,
            high:                         20080.0,
            low:                          19800.0,
            close:                        19980.0,
            volume:                       0.0,
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     false,
        };

        // println!("{:?}", Classic::new(&c));
        // println!("{:#?}", Woodie::new(&c));
        // println!("{:#?}", Camarilla::new(&c));
        // println!("{:#?}", Fibonacci::new(&c));
        // println!("{:#?}", DeMark::new(&c));

        let mut p = Fibonacci::new(&c);
        /*
        println!("{:#?}", p.level(-4).unwrap());
        println!("{:#?}", p.level(-3).unwrap());
        println!("{:#?}", p.level(-2).unwrap());
        println!("{:#?}", p.level(-1).unwrap());
        println!("{:#?}", p.level(1).unwrap());
        println!("{:#?}", p.level(2).unwrap());
        println!("{:#?}", p.level(3).unwrap());
        println!("{:#?}", p.level(4).unwrap());
        println!("{:#?}", p);
         */

        println!(
            "{:<5.8?}  {:<5.8?}  {:<5.8?}  {:<5.8?}     {:<5.8?}     {:>5.8?}  {:>5.8?}  {:>5.8?}  {:>5.8?}",
            p.level(-4).unwrap(),
            p.level(-3).unwrap(),
            p.level(-2).unwrap(),
            p.level(-1).unwrap(),
            p.pp(),
            p.level(1).unwrap(),
            p.level(2).unwrap(),
            p.level(3).unwrap(),
            p.level(4).unwrap()
        );

        for x in (19755..21780).step_by(1) {
            let price = x as ValueType * 1.00;
            match p.price_to_index(price) {
                None => {
                    println!("P: {:0.2?} {:0.2?}", price, p.index_to_price(price));
                }
                Some(loc) => {
                    println!("P: {:0.2?} L(P): {:0.2?} P(L): {:0.2?}", price, loc, p.index_to_price(loc).unwrap());
                }
            }
        }

        /*
        for x in (2010..2200).step_by(1) {
            let x = x as ValueType * 0.10;
            println!("{:0.2?} = {:0.2?}", x, p.locate(x));
        }
         */
    }
}
