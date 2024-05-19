use crate::{
    appraiser::Appraiser,
    candle::Candle,
    exchange_client::TradeSignal,
    state_indicator::ComputedIndicatorState,
    state_moving_average::ComputedMovingAverageState,
    support_resistance::SRLevel,
    timeframe::{Timeframe, TimeframeError},
    timeset::TimeSet,
    types::{Direction, Gauge, IndicationStatus, Motion, OrderedValueType},
    util::{calc_ratio_x_to_y, calc_relative_position_between, gauge_location, round_float_to_ordered_precision},
};
use anyhow::{Context, Result};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use std::{fmt::Debug, time::Duration};
use thiserror::Error;
use yata::core::{PeriodType, Window};

pub type PriceLevel = (usize, &'static str, OrderedValueType);

#[derive(Debug)]
pub struct PriceRange {
    pub lower:             SRLevel,
    pub upper:             SRLevel,
    pub distance:          (OrderedValueType, OrderedValueType),
    pub distance_to_lower: (OrderedValueType, OrderedValueType),
    pub distance_to_upper: (OrderedValueType, OrderedValueType),
    pub relative_position: (OrderedValueType, Gauge),
}

impl PriceRange {
    pub fn new(appraiser: &Appraiser, price: OrderedValueType, lower: SRLevel, upper: SRLevel) -> Self {
        let distance = appraiser.round_quote((upper.value - lower.value).0).into();
        let distance_pc = round_float_to_ordered_precision(calc_ratio_x_to_y(distance, price.0), 5);

        let distance_to_lower = appraiser.round_quote((lower.value - price).abs()).into();
        let distance_to_lower_pc = round_float_to_ordered_precision(calc_ratio_x_to_y(distance_to_lower, price.0), 5);

        let distance_to_upper = appraiser.round_quote((upper.value - price).abs()).into();
        let distance_to_upper_pc = round_float_to_ordered_precision(calc_ratio_x_to_y(distance_to_upper, price.0), 5);

        let relative_position = round_float_to_ordered_precision(calc_relative_position_between(price.0, lower.value.0, upper.value.0), 2);

        Self {
            lower,
            upper,
            distance: (OrderedFloat(distance), distance_pc),
            distance_to_lower: (OrderedFloat(distance_to_lower), distance_to_lower_pc),
            distance_to_upper: (OrderedFloat(distance_to_upper), distance_to_upper_pc),
            relative_position: (relative_position, gauge_location(relative_position)),
        }
    }
}

#[derive(Error, Debug)]
pub enum AnalyzerError {
    #[error("Invalid index {0}")]
    InvalidIndex(usize),

    #[error("Invalid lookback index {1} for timeframe {0:?}")]
    InvalidTimeframeOrIndex(Timeframe, PeriodType),

    #[error("Computed state {1} for timeframe {0:?} not found")]
    StateNotFound(Timeframe, PeriodType),

    #[error("Candle {1} for timeframe {0:?} not found")]
    CandleNotFound(Timeframe, PeriodType),

    #[error(transparent)]
    TimeframeError(#[from] TimeframeError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Default)]
pub struct IndicationMetadata {
    value:     [OrderedValueType; 2],
    direction: [Direction; 2],
    motion:    [Motion; 2],
    status:    IndicationStatus,
}

impl IndicationMetadata {
    pub fn value(&self, index: usize) -> Result<OrderedValueType, AnalyzerError> {
        match self.value.get(index) {
            None => Err(AnalyzerError::InvalidIndex(index)),
            Some(&value) => Ok(value),
        }
    }

    pub fn direction(&self, index: usize) -> Result<Direction, AnalyzerError> {
        match self.direction.get(index) {
            None => Err(AnalyzerError::InvalidIndex(index)),
            Some(&direction) => Ok(direction),
        }
    }

    pub fn motion(&self, index: usize) -> Result<Motion, AnalyzerError> {
        match self.motion.get(index) {
            None => Err(AnalyzerError::InvalidIndex(index)),
            Some(&motion) => Ok(motion),
        }
    }
}

#[derive(Debug)]
pub struct AnalyzerSummary {
    pub states:           Vec<ComputedIndicatorState>,
    pub price:            IndicationMetadata,
    pub rsi:              IndicationMetadata,
    pub kst:              IndicationMetadata,
    pub cmf:              IndicationMetadata,
    pub mfi:              IndicationMetadata,
    pub efi:              IndicationMetadata,
    pub kama:             IndicationMetadata,
    pub macd:             IndicationMetadata,
    pub atr:              IndicationMetadata,
    pub min_weight:       usize,
    pub total_range:      PriceRange,
    pub local_range:      Option<PriceRange>,
    pub computation_time: Duration,
}

impl AnalyzerSummary {
    pub fn signal(&self) -> TradeSignal {
        use Direction::*;
        use Motion::*;

        /*
        let v0 = (self.rsi.direction(0).unwrap(), self.rsi.motion(1).unwrap());
        let v1 = (self.rsi.direction(1).unwrap(), self.rsi.motion(1).unwrap());

        if self.rsi.value(0).unwrap() <= OrderedFloat(0.10) {
            return TradeSignal::Buy;
        } else if self.rsi.value(0).unwrap() >= OrderedFloat(0.85) {
            return TradeSignal::Sell;
        }
         */

        /*
        let mfi_is_below_lower_threshold = self.states.iter().rev().take(3).all(|s| OrderedFloat(s.mfi.value(1)) <= OrderedFloat(0.05));
        let mfi_is_above_upper_threshold = self.states.iter().rev().take(3).all(|s| OrderedFloat(s.mfi.value(1)) >= OrderedFloat(0.95));

        if self.cmf.value(0).unwrap() < OrderedFloat(0.0) && mfi_is_below_lower_threshold {
            return TradeSignal::Buy;
        } else if self.cmf.value(0).unwrap() > OrderedFloat(0.0) && mfi_is_above_upper_threshold {
            return TradeSignal::Sell;
        }
         */

        /*
        if self.mfi.value(0).unwrap() <= OrderedFloat(0.15) {
            return TradeSignal::Buy;
        } else if self.mfi.value(0).unwrap() >= OrderedFloat(0.80) {
            return TradeSignal::Sell;
        }
         */

        /*
        // if v1 == (Falling, Decelerating) && self.kst.value(0).unwrap() <= OrderedFloat(-0.0025) {
        if self.kst.motion(1).unwrap() == Decelerating && self.kst.value(0).unwrap() >= OrderedFloat(0.0020) {
            return TradeSignal::Buy;
        // } else if v1 == (Rising, Decelerating) && self.kst.value(0).unwrap() >= OrderedFloat(0.0025) {
        } else if v1 == (Rising, Decelerating) {
            return TradeSignal::Sell;
        }
         */

        TradeSignal::None
    }
}

pub struct Analyzer<'a> {
    appraiser:                      &'a Appraiser,
    price:                          OrderedValueType,
    candles:                        &'a TimeSet<(Candle, Window<Candle>)>,
    computed_indicator_states:      &'a TimeSet<(ComputedIndicatorState, Window<ComputedIndicatorState>)>,
    computed_moving_average_states: &'a TimeSet<(ComputedMovingAverageState, Window<ComputedMovingAverageState>)>,
}

impl<'a> Analyzer<'a> {
    /// WARNING: `levels` must be sorted by price in ascending order
    pub fn new(
        appraiser: &'a Appraiser,
        price: OrderedValueType,
        candles: &'a TimeSet<(Candle, Window<Candle>)>,
        computed_indicator_states: &'a TimeSet<(ComputedIndicatorState, Window<ComputedIndicatorState>)>,
        computed_moving_average_states: &'a TimeSet<(ComputedMovingAverageState, Window<ComputedMovingAverageState>)>,
    ) -> Self {
        Self {
            appraiser,
            price,
            candles,
            computed_indicator_states,
            computed_moving_average_states,
        }
    }

    /*
    pub fn surrounding_levels(
        &self,
        timeframe: Timeframe,
        index: PeriodType,
        min_weight: usize,
        min_distance_pc: OrderedValueType,
    ) -> Result<Option<(SRLevel, SRLevel)>, AnalyzerError> {
        let state = self.ma_state(timeframe, index)?;

        let Some(level_above_price) = state
            .into_iter()
            .filter(|sr| sr.weight >= min_weight)
            .find_position(|sr| sr.price > self.price) else {
            return Ok(None);
        };

        let Some(level_below_price) = state
            .into_iter()
            .rev()
            .filter(|sr| sr.weight >= min_weight && OrderedFloat((sr.price - level_above_price.1.price).abs()) >= min_distance_pc)
            .find_position(|sr| sr.price < self.price) else {
            return Ok(None);
        };

        Ok(match (level_below_price, level_above_price) {
            ((pos_below, level_below_price), (pos_above, level_above_price)) => {
                let level_below_price = (pos_below, level_below_price.name, level_below_price.price);
                let level_above_price = (pos_above, level_above_price.name, level_above_price.price);

                Some((level_below_price, level_above_price))
            }

            _ => None,
        })
    }
     */

    /*
    pub fn analyze(
        &self,
        timeframe: Timeframe,
        index: PeriodType,
        lookback_length: PeriodType,
        min_weight: usize,
        min_distance_pc: OrderedValueType,
    ) -> Result<Option<AnalyzerSummary>, AnalyzerError> {
        let it = Instant::now();

        let states = self.states(timeframe, index, lookback_length)?;
        let ma_states = self.ma_states(timeframe, index, lookback_length)?;

        let Some(head_state) = states.last() else {
            return Ok(None);
        };

        let Some(head_ma_state) = ma_states.last() else {
            return Ok(None);
        };

        let prices = states.iter().map(|s| s.candle.close).collect_vec();
        let atrs = states.iter().map(|s| s.atr_13).collect_vec();

        let mut price = IndicationMetadata::default();
        price.value[0] = OrderedFloat(head_state.candle.close);
        price.direction[0] = direction(prices.as_slice(), 4, 0);
        price.motion[0] = motion(prices.as_slice(), 4);

        let mut atr = IndicationMetadata::default();
        atr.value[0] = OrderedFloat(head_state.atr_13);
        atr.value[1] = OrderedFloat(calc_x_is_percentage_of_y(head_state.atr_13, head_state.candle.close));
        atr.direction[0] = direction(atrs.as_slice(), 4, 0);
        atr.motion[0] = motion(atrs.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // rsi

        let rsis = states.iter().map(|s| s.rsi_14.value(0)).collect_vec();
        let rsi_mas = states.iter().map(|s| s.rsi_14_wma_14).collect_vec();

        let mut rsi = IndicationMetadata::default();
        rsi.value[0] = OrderedFloat(head_state.rsi_14.value(0));
        rsi.value[1] = OrderedFloat(head_state.rsi_14_wma_14);
        rsi.direction[0] = direction(rsis.as_slice(), 4, 0);
        rsi.direction[0] = direction(rsi_mas.as_slice(), 4, 0);
        rsi.motion[0] = motion(rsis.as_slice(), 4);
        rsi.motion[0] = motion(rsi_mas.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // know sure thing

        let ksts = states.iter().map(|s| s.kst.value(0)).collect_vec();
        let kst_signals = states.iter().map(|s| s.kst.value(1)).collect_vec();

        let mut kst = IndicationMetadata::default();
        kst.value[0] = OrderedFloat(head_state.kst.value(0));
        kst.value[1] = OrderedFloat(head_state.kst.value(1));
        kst.direction[0] = direction(ksts.as_slice(), 4, 0);
        kst.direction[1] = direction(kst_signals.as_slice(), 4, 0);
        kst.motion[0] = motion(ksts.as_slice(), 4);
        kst.motion[1] = motion(kst_signals.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // chaikin money flow

        let cmfs = states.iter().map(|s| s.cmf.value(0)).collect_vec();
        let mut cmf = IndicationMetadata::default();
        cmf.value[0] = OrderedFloat(head_state.cmf.value(0));
        cmf.direction[0] = direction(cmfs.as_slice(), 4, 0);
        cmf.motion[0] = motion(cmfs.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // MACD

        let macds = states.iter().map(|s| s.macd.value(0)).collect_vec();
        let macd_signals = states.iter().map(|s| s.macd.value(1)).collect_vec();

        let mut macd = IndicationMetadata::default();
        macd.value[0] = OrderedFloat(head_state.macd.value(0));
        macd.value[1] = OrderedFloat(head_state.macd.value(1));
        macd.direction[0] = direction(macds.as_slice(), 4, 0);
        macd.direction[1] = direction(macds.as_slice(), 4, 0);
        macd.motion[0] = motion(macds.as_slice(), 4);
        macd.motion[1] = motion(macd_signals.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // kaufman

        let kamas = states.iter().map(|s| s.kama.value(0)).collect_vec();
        let mut kama = IndicationMetadata::default();
        kama.value[0] = OrderedFloat(head_state.kama.value(0));
        kama.direction[0] = direction(kamas.as_slice(), 4, 0);
        kama.motion[0] = motion(kamas.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // elder's force index

        let efis = states.iter().map(|s| s.efi.value(0)).collect_vec();
        let mut efi = IndicationMetadata::default();
        efi.value[0] = OrderedFloat(head_state.efi.value(0));
        efi.direction[0] = direction(efis.as_slice(), 4, 0);
        efi.motion[0] = motion(efis.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // money flow index

        let mfis = states.iter().map(|s| s.mfi.value(1)).collect_vec();
        let mut mfi = IndicationMetadata::default();
        mfi.value[0] = OrderedFloat(head_state.mfi.value(1));
        mfi.direction[0] = direction(mfis.as_slice(), 4, 0);
        mfi.motion[0] = motion(mfis.as_slice(), 4);

        // ----------------------------------------------------------------------------
        // levels and ranges

        let n_levels = head_ma_state.len();
        let (lowest_level, highest_level) = head_ma_state.minmax();

        /*
        let local_range = match self.surrounding_levels(timeframe, index, min_weight, min_distance_pc)? {
            None => None,
            Some((level_below, level_above)) => Some(PriceRange::new(self.appraiser, self.price, level_below, level_above)),
        };
         */

        let local_range = None;

        // ----------------------------------------------------------------------------
        // know sure thing

        let ksts = states.iter().map(|s| s.kst.value(0)).collect_vec();
        let kst_signals = states.iter().map(|s| s.kst.value(1)).collect_vec();

        Ok(Some(AnalyzerSummary {
            states,
            price,
            rsi,
            kst,
            cmf,
            mfi,
            efi,
            kama,
            macd,
            atr,
            min_weight,
            total_range: PriceRange::new(self.appraiser, self.price, lowest_level, highest_level),
            local_range,
            computation_time: it.elapsed(),
        }))
    }
     */
}

#[cfg(test)]
mod tests {
    /*
    #[test]
    fn test_analyzer() {
        let appraiser = Appraiser::new(rules_for_testing());
        let snapshot = Snapshot::default();
        let price = OrderedFloat(1.418);
        let levels = [1.0, 0.5, 1.5, 2.5, 2.3, 2.1, 1.3, 3.1]
            .into_iter()
            .map(|x| OrderedFloat(x))
            .sorted_by(|a, b| a.cmp(b))
            .enumerate()
            .map(|(w, x)| SRLevel { price: x, weight: w, name: "" })
            .collect_vec();

        let it = Instant::now();
        let a = Analyzer::new(&appraiser, price, &snapshot, levels.as_slice());

        // println!("{:#?}", a.surrounding_levels(2));

        let summary = a.analyze(0, OrderedFloat(0.0001));
        println!("{:#?}", it.elapsed());

        println!("{:#?}", summary);
    }
     */

    /*
    #[test]
    fn test_surrounding_levels() {
        let price = OrderedFloat(1.618);
        let levels = [1.0, 0.5, 1.5, 2.5, 2.3, 2.1, 1.3, 3.1]
            .into_iter()
            .enumerate()
            .map(|(w, x)| SRLevel {
                price: OrderedFloat(x),
                weight: w,
                name: "",
            })
            .collect_vec();

        match surrounding_levels(price, levels.as_slice(), 2) {
            None => {}
            Some((below, above)) => {
                let relpos = calc_relative_position_between(price.0, below.price.0, above.price.0);

                println!("{:#?}", relpos);
            }
        }
    }
     */
}
