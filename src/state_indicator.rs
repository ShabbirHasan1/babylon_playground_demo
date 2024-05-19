use crate::{
    candle::Candle,
    config::{CoinConfig, TimeframeConfig},
    support_resistance::SRLevel,
    timeframe::Timeframe,
    timeset::TimeSet,
};
use anyhow::{bail, Result};
use itertools::Itertools;
use log::info;
use ordered_float::OrderedFloat;
use std::time::{Duration, Instant};
use yata::{
    core::{Action, IndicatorConfig, IndicatorInstance, IndicatorResult, Method, MovingAverageConstructor, PeriodType, Source, ValueType, Window},
    helpers::{MAInstance, MA},
    methods::{st_dev_variance_sqrt::StDevVarianceSqrt, TR},
};

#[derive(Copy, Clone, Debug)]
pub struct ComputedIndicatorState {
    // FIXME: Candle inside the state is redundant, but it's here for now
    pub candle:           Candle,
    pub ema_3:            ValueType,
    pub ema_3_stdev:      ValueType,
    pub ema_5:            ValueType,
    pub ema_5_stddev:     ValueType,
    pub ema_8:            ValueType,
    pub ema_8_stddev:     ValueType,
    pub ema_13:           ValueType,
    pub ema_13_stddev:    ValueType,
    pub ema_21:           ValueType,
    pub ema_21_stddev:    ValueType,
    pub rsi_3:            IndicatorResult,
    pub rsi_14:           IndicatorResult,
    pub tr:               ValueType,
    pub tr_roc:           ValueType,
    pub stdev_tr_ln:      ValueType,
    pub stdev_tr_roc:     ValueType,
    pub stdev_high_low:   ValueType,
    pub stdev_open_close: ValueType,
    pub bb:               IndicatorResult,
    pub kc:               IndicatorResult,
    pub kama:             IndicatorResult,
    pub kst:              IndicatorResult,
    pub psar_scalp:       IndicatorResult,
    pub psar_trend:       IndicatorResult,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct ComputedPersistentIndicatorState {
    pub ema_3:                   ValueType,
    pub ema_3_stdev:             ValueType,
    pub ema_5:                   ValueType,
    pub ema_5_stddev:            ValueType,
    pub ema_8:                   ValueType,
    pub ema_8_stddev:            ValueType,
    pub ema_13:                  ValueType,
    pub ema_13_stddev:           ValueType,
    pub ema_21:                  ValueType,
    pub ema_21_stddev:           ValueType,
    pub ema_21_cross:            Action,
    pub kc:                      IndicatorResult,
    pub bb:                      IndicatorResult,
    pub kama:                    IndicatorResult,
    pub kst:                     IndicatorResult,
    pub psar_scalp:              IndicatorResult,
    pub psar_trend:              IndicatorResult,
    pub linvol_5_close_baseline: ValueType,
    pub linvol_5_close:          ValueType,
}

impl Default for ComputedIndicatorState {
    fn default() -> Self {
        Self {
            candle:           Default::default(),
            ema_3:            0.0,
            ema_3_stdev:      0.0,
            ema_5:            0.0,
            ema_5_stddev:     0.0,
            ema_8:            0.0,
            ema_8_stddev:     0.0,
            ema_13:           0.0,
            ema_13_stddev:    0.0,
            ema_21:           0.0,
            ema_21_stddev:    0.0,
            rsi_3:            Default::default(),
            rsi_14:           Default::default(),
            tr:               0.0,
            tr_roc:           0.0,
            stdev_tr_ln:      0.0,
            stdev_tr_roc:     0.0,
            stdev_high_low:   0.0,
            stdev_open_close: 0.0,
            kc:               Default::default(),
            bb:               Default::default(),
            kama:             Default::default(),
            kst:              Default::default(),
            psar_scalp:       Default::default(),
            psar_trend:       Default::default(),
        }
    }
}
