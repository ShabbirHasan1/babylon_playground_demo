use std::{
    ops::{Deref, DerefMut},
    time::{Duration as StdDuration, Instant},
};

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use egui::plot::CandleElem;
use itertools::Itertools;
use log::{debug, error, info, warn};
use ordered_float::OrderedFloat;
use thiserror::Error;
use ustr::{ustr, Ustr};
use yata::{
    core::{Action, MovingAverage, MovingAverageConstructor, PeriodType, Source},
    helpers::{MAInstance, MA},
    indicators::{BollingerBands, KeltnerChannel, KnowSureThing, ParabolicSAR, RelativeStrengthIndex, KAMA, MACD},
    methods::{st_dev_variance_sqrt::StDevVarianceSqrt, CollapseTimeframe, Conv, Cross, LinearVolatility, Momentum, RateOfChange, TR},
    prelude::*,
};

use crate::{
    candle::Candle,
    f,
    id::CoinPairId,
    model::ExchangeTradeEvent,
    series::Series,
    sliding_window::SlidingWindow,
    state_indicator::{ComputedIndicatorState, ComputedPersistentIndicatorState},
    state_moving_average::{ComputedMovingAverageState, FrozenMovingAverageState},
    support_resistance::MaType,
    timeframe::Timeframe,
    types::OrderedValueType,
    util::FIB_SEQUENCE,
};

#[derive(Debug, Clone, Error)]
pub enum CandleSeriesError {
    #[error("Candle not found")]
    CandleNotFound,

    #[error("Working candle not found")]
    WorkingCandleNotFound,

    #[error("Working state not found")]
    WorkingStateNotFound,

    #[error("Trade-only series cannot process candles")]
    ComputeTradeOnlySeries,

    #[error("Invalid timeframe: given {given:?}, expected {expected:?}")]
    InvalidTimeframe { given: Timeframe, expected: Timeframe },

    #[error("Invalid symbol: given={0}, expected={1}")]
    InvalidSymbol(Ustr, Ustr),

    #[error("Trade not within the current candle timeframe")]
    NotWithinCandleTimeframe {
        trade_time: DateTime<Utc>,
        open_time:  DateTime<Utc>,
        close_time: DateTime<Utc>,
    },

    #[error("Cannot push base candle without a base timeframe")]
    CannotPushBaseCandleWithoutBaseTimeframe,

    #[error("Not enough candles to compute the working state")]
    NotEnoughCandles,

    #[error(transparent)]
    YataError(yata::core::Error),
}

/// Calculate the start time of the first candle based on the trade's timestamp.
fn calculate_candle_start_time(trade_time: DateTime<Utc>, timeframe_duration_ms: i64) -> DateTime<Utc> {
    // Find today's midnight relative to the trade time
    let midnight = Utc.with_ymd_and_hms(trade_time.year(), trade_time.month(), trade_time.day(), 0, 0, 0).unwrap();

    // Calculate the number of milliseconds from midnight to the trade time
    let millis_since_midnight = trade_time.timestamp_millis() - midnight.timestamp_millis();

    // Calculate the number of complete timeframes that have passed since midnight
    let complete_timeframes = millis_since_midnight / timeframe_duration_ms;

    // Calculate the start time of the current timeframe
    let current_timeframe_start_millis = midnight.timestamp_millis() + (complete_timeframes * timeframe_duration_ms);

    // Convert the milliseconds back to a DateTime<Utc>
    Utc.timestamp_millis_opt(current_timeframe_start_millis).unwrap()
}

#[derive(Debug, Clone)]
pub struct ManagedCandle {
    pub candle: Candle,
    pub ui:     Option<CandleElem>,
    pub state:  ComputedState,
}

impl ManagedCandle {
    #[inline]
    pub fn new(candle: Candle, state: ComputedState, with_ui: bool) -> Self {
        Self {
            candle,
            ui: if with_ui {
                Some(CandleElem {
                    x:             0.0,
                    candle:        egui::plot::Candle {
                        timeframe:                    0,
                        open_time:                    Default::default(),
                        close_time:                   Default::default(),
                        open:                         0.0,
                        high:                         0.0,
                        low:                          0.0,
                        close:                        0.0,
                        volume:                       0.0,
                        number_of_trades:             0,
                        quote_asset_volume:           0.0,
                        taker_buy_quote_asset_volume: 0.0,
                        taker_buy_base_asset_volume:  0.0,
                    },
                    candle_width:  0.0,
                    whisker_width: 0.0,
                    stroke:        Default::default(),
                    fill:          Default::default(),
                })
            } else {
                None
            },
            state,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkingState {
    pub i:  IndicatorState,
    pub p:  PersistentIndicatorState,
    pub ma: Vec<(MaType, usize, MAInstance)>,
}

/// The state of the series' indicators
#[derive(Debug, Clone)]
pub struct IndicatorState {
    pub ema_3:            MAInstance,
    pub ema_3_stdev:      StDevVarianceSqrt,
    pub ema_5:            MAInstance,
    pub ema_5_stdev:      StDevVarianceSqrt,
    pub ema_8:            MAInstance,
    pub ema_8_stdev:      StDevVarianceSqrt,
    pub ema_13:           MAInstance,
    pub ema_13_stdev:     StDevVarianceSqrt,
    pub ema_21:           MAInstance,
    pub ema_21_stdev:     StDevVarianceSqrt,
    pub rsi_3:            <RelativeStrengthIndex as IndicatorConfig>::Instance,
    pub rsi_14:           <RelativeStrengthIndex as IndicatorConfig>::Instance,
    pub bb:               <BollingerBands as IndicatorConfig>::Instance,
    pub kc:               <KeltnerChannel as IndicatorConfig>::Instance,
    pub kama:             <KAMA as IndicatorConfig>::Instance,
    pub macd:             <MACD as IndicatorConfig>::Instance,
    pub kst:              <KnowSureThing as IndicatorConfig>::Instance,
    pub tr:               TR,
    pub tr_roc:           RateOfChange,
    pub psar_scalp:       <ParabolicSAR as IndicatorConfig>::Instance,
    pub psar_trend:       <ParabolicSAR as IndicatorConfig>::Instance,
    pub stdev_tr_ln:      StDevVarianceSqrt,
    pub stdev_tr_roc:     StDevVarianceSqrt,
    pub stdev_high_low:   StDevVarianceSqrt,
    pub stdev_open_close: StDevVarianceSqrt,
}

#[derive(Debug, Clone)]
pub struct PersistentIndicatorState {
    pub ema_21:         MAInstance,
    pub ema_21_cross:   Cross,
    pub bb:             <BollingerBands as IndicatorConfig>::Instance,
    pub kc:             <KeltnerChannel as IndicatorConfig>::Instance,
    pub kama:           <KAMA as IndicatorConfig>::Instance,
    pub macd:           <MACD as IndicatorConfig>::Instance,
    pub psar_scalp:     <ParabolicSAR as IndicatorConfig>::Instance,
    pub psar_trend:     <ParabolicSAR as IndicatorConfig>::Instance,
    pub linvol_5_close: LinearVolatility,
}

/// Computed state for a single candle
#[derive(Debug, Clone, Default)]
pub struct ComputedState {
    pub candle:                          Candle,
    pub i:                               ComputedIndicatorState,
    pub p:                               ComputedPersistentIndicatorState,
    pub ma:                              Vec<(MaType, usize, OrderedValueType)>,
    pub indicator_computation_time:      StdDuration,
    pub moving_average_computation_time: StdDuration,
    pub total_computation_time:          StdDuration,
}

impl ComputedState {
    #[inline]
    pub fn price(&self) -> OrderedValueType {
        f!(self.candle.close)
    }
}

/// # Algorithm Description
///
/// ## Introduction
///
/// The process involves aggregating lower-timeframe candles (base timeframe) into a higher-timeframe candle (series
/// timeframe), suitable for scenarios like constructing 5-minute candles from 1-minute candles.
///
/// ## Terminology
///
/// - **Series**: An ordered collection of candles from tail (oldest) to head (newest). Indexed with 0 for the head,
///   increasing towards the tail.
/// - **New Candle**: The latest incoming candle data snapshot. Represents complete data (open, high, low, close, etc.), not
///   incremental changes.
/// - **Working Index**: Tracks the count of lower-timeframe candles aggregated towards the next higher-timeframe candle in
///   the series. Incremented with each new push.
/// - **Working Index Goal**: The required count of lower-timeframe candles to complete an aggregated higher-timeframe
///   candle.
/// - **Working Candle**: The current state of aggregation, representing the latest higher-timeframe candle being
///   constructed.
/// - **Working Candle Backup**: A snapshot of the working candle's last finalized state, used as a base for further
///   aggregation until the working candle is finalized.
///
/// ## Process Overview
///
/// ### Initialization for First Candle Push
///
/// #### Non-final Candle:
///
/// If there is no working candle, it is created using the new candle's data. This includes setting the initial state that
/// will be aggregated upon by subsequent candles. Similarly, if there is no working candle backup, create one using the
/// same initial data. This backup will hold the state of aggregated final candles and is updated only when processing final
/// candles.
///
/// #### Final Candle:
///
/// If the first candle is final, it's treated similarly by initializing the working candle and the working candle backup,
/// but also increment the working index as this counts towards the completion of an aggregated candle.
///
/// ### Subsequent Candle Pushes
///
/// For Each New Candle (Final or Non-final): Aggregate its data with the data from the working candle backup to update the
/// working candle. This ensures the working candle always reflects the latest state based on previously finalized candles
/// and the new incoming data. If the new candle is final, the working candle backup is also updated with this new
/// aggregated state, ensuring it represents the latest finalized aggregation. Increment the Working Index only if the new
/// candle is final. This correctly tracks progress towards the working index goal with respect to final candles only.
///
/// ### Finalization and Series Integration
///
/// Once the working index matches the working index goal, or a final candle's arrival necessitates the finalization: Mark
/// the working candle as final. Push the working candle to the series as a new, completed higher-timeframe candle. Reset
/// the working candle, working candle backup, and working index to start the next cycle of aggregation. This prepares for
/// constructing the next higher-timeframe candle.
///
/// ## Handling Subsecond Timeframes
///
/// ### Initial Setup
///
/// - **Series Initialization**: On receiving the first trade, calculate its offset from the midnight anchor to determine
///   the start time for the first candle in the series. This becomes the reference point for subsequent candles.
/// - **First Candle Creation**: Use the first trade to create the initial candle, aligning it with the calculated start
///   time.
///
/// ### Subsequent Trade Processing
///
/// - **Using the Latest Candle as a Reference**: For each new trade, reference the latest candle in the series to determine
///   the next steps.
///     - If the trade falls within the current candle's timeframe, update the candle with the trade's data.
///     - If the trade indicates a new timeframe, use gap filling as needed before creating a new candle.
///
/// ### Gap Filling
///
/// - **Detecting Gaps**: Compare the timestamp of the incoming trade with the close time of the latest candle. If there's a
///   gap larger than the expected timeframe duration, fill it with filler candles.
/// - **Creating Filler Candles**: For each gap, create filler candles to maintain series continuity. Each filler candle's
///   open, high, low, and close values match the last known close price, with volume set to zero.
#[derive(Debug, Clone)]
pub struct CandleSeries {
    /// The symbol of the series
    symbol: Ustr,

    /// The base timeframe of the series which is used to aggregate into the main timeframe (if different)
    base_timeframe: Timeframe,

    /// The main timeframe of the series. The series is constructed using this timeframe.
    /// If the base timeframe is different, it is used to aggregate into this timeframe.
    timeframe: Timeframe,

    /// The series of candles
    candles: Series<Candle>,

    /// The series of computed states for each candle in the series (excluding the working candle, because
    /// the working candle's state is stored in the working_state field, similarly to the working_candle logic)
    states: Series<ComputedState>,

    /// The goal for the working index to reach before finalizing the working candle and pushing it into the series.
    working_index_goal: usize,

    /// The current working index, tracking the count of base timeframe candles aggregated into the working candle.
    working_index: usize,

    /// The current working candle, representing the state of the main timeframe candle being constructed.
    /// IMPORTANT: Unlike the series' candles, the working candle is not included in the series until it is finalized.
    working_candle: Option<Candle>,

    /// The UI representation of the working candle (if required)
    working_candle_ui: Option<CandleElem>,

    /// The backup of the working candle's last finalized state, used as a base for further
    /// aggregation until the working candle is finalized.
    working_candle_backup: Option<Candle>,

    /// The current state of the series' indicators
    /// IMPORTANT: The working state is used to store the current state of the indicators and moving averages
    /// IMPORTANT: The computed state of the working candle is stored in the `self.states` series, as a head element
    working_state: Option<WorkingState>,

    /// When a new candle update is received that is ahead of the working candle,
    /// it is stored here until the working candle is finalized and computed state is pushed into the series.
    next_candle_backup: Option<Candle>,

    /// If true, the series will only process trades and ignore full candles.
    trades_only: bool,

    /// If true, the series will fill gaps in the past with filler candles based on the last candle's close price.
    fill_gaps: bool,

    /// If true, the series is live and will process incoming candle updates and trades in real-time.
    pub is_live: bool,

    /// If true, the series will generate UI elements for each candle
    /// IMPORTANT: This is useful for debugging and visualization purposes
    generate_ui: bool,

    /// The last trade time processed by the series
    last_trade_time: DateTime<Utc>,
}

impl CandleSeries {
    /// Create a new candle series with the given parameters.
    ///
    /// `symbol` - The symbol of the series
    /// `timeframe` - The main timeframe of the series. The series is constructed using this timeframe.
    /// `base_timeframe` - The base timeframe of the series which is used to aggregate into the main timeframe (if different)
    /// `capacity` - The capacity of the series
    /// `period` - The period of the series (used for indicators and moving averages)
    /// `fill_gaps` - If true, the series will fill gaps in the past with filler candles based on the last candle's close price.
    /// `trades_only` - If true, the series will only process trades and ignore full candles.
    /// `generate_ui` - If true, the series will generate UI elements for each candle
    ///
    /// IMPORTANT:
    /// - The series is constructed using the main timeframe, and the base timeframe is used to aggregate into the main timeframe (if different)
    /// - The series is not live by default, and will not process incoming candle updates and trades in real-time.
    /// - The series will not generate UI elements for each candle by default.
    /// - The series will not fill gaps in the past with filler candles by default.
    /// - The series will process full candles by default.
    pub fn new(
        symbol: Ustr,
        timeframe: Timeframe,
        base_timeframe: Option<Timeframe>,
        capacity: usize,
        period: usize,
        fill_gaps: bool,
        trades_only: bool,
        generate_ui: bool,
    ) -> Self {
        let trades_only = matches!(timeframe, Timeframe::MS100 | Timeframe::MS250 | Timeframe::MS500) || trades_only;
        let base_timeframe = base_timeframe.unwrap_or(timeframe);
        let working_index_goal = if base_timeframe != timeframe {
            // Calculate the ratio between the base and main timeframe durations to determine how many base timeframe units
            // are needed to aggregate into a single main timeframe unit.
            let base_duration_ms = base_timeframe.duration().num_milliseconds();
            let main_duration_ms = timeframe.duration().num_milliseconds();
            (main_duration_ms / base_duration_ms) as usize
        } else {
            1 // If the base timeframe matches the main timeframe, no aggregation is needed.
        };

        CandleSeries {
            symbol,
            base_timeframe,
            timeframe,
            candles: Series::new(capacity),
            states: Series::new(capacity),
            working_index_goal: working_index_goal,
            working_index: 0,
            working_candle: None,
            working_candle_ui: None,
            working_candle_backup: None,
            working_state: None,
            next_candle_backup: None,
            trades_only,
            fill_gaps,
            is_live: false,
            generate_ui,
            last_trade_time: Default::default(),
        }
    }

    pub fn price(&self) -> OrderedValueType {
        self.head_state().map(|s| s.price()).unwrap_or_else(|| {
            warn!("No price available, returning 0.0");
            f!(0.0)
        })
    }

    // TODO: Implement a method to initialize the series with a first candle
    // FIXME: This method should be implemented along with initialize_first_trade
    pub fn initialize_first_candle(&mut self, first_candle: Option<Candle>) -> Result<(), CandleSeriesError> {
        // If the first candle is provided, validate it and push it into the series.
        if let Some(first_candle) = first_candle {
            if first_candle.timeframe != self.timeframe {
                return Err(CandleSeriesError::InvalidTimeframe {
                    given:    first_candle.timeframe,
                    expected: self.timeframe,
                });
            }

            if first_candle.symbol != self.symbol {
                return Err(CandleSeriesError::InvalidSymbol(first_candle.symbol, self.symbol));
            }

            // If the first candle is final, push it into the series. Otherwise, set it as the working candle.
            if first_candle.is_final {
                self.push_candle(first_candle)?;
            } else {
                self.working_candle = Some(first_candle.clone());
                self.working_candle_backup = Some(first_candle.clone());
            }

            return Ok(());
        }

        Ok(())
    }

    // FIXME: Do not compute indicators manually inside `initialize_working_state`, let `push_candle` do it for you.
    pub fn initialize_working_state(&mut self) -> Result<(), CandleSeriesError> {
        if self.num_candles() < 3 {
            return Err(CandleSeriesError::NotEnoughCandles);
        }

        // Collect the candles in reverse order to initialize the indicators and moving averages
        // using the most recent data first.
        // NOTE: The order is from the oldest to the newest.
        let candles = self.candles.iter_rev().cloned().collect_vec();
        let mut states = Series::<ComputedState>::new(candles.len());

        // First candle is the oldest, second candle is the second oldest, and so on.
        let c0 = candles.get(0).cloned().expect("Failed to get the first candle");
        let c1 = candles.get(1).cloned().expect("Failed to get the second candle");

        // IMPORTANT: `history_length` is the size of the resulting series after all the calculations.
        // IMPORTANT: `max_period` is taken from number of available candles, not the series capacity.
        // let max_period = candles.len() as PeriodType + 1;
        let max_period = self.capacity() + 1;

        // ----------------------------------------------------------------------------
        // Preparing the first states
        // NOTE: Instance variables names are re-used to store the computed values
        // IMPORTANT: First, computing the first indicators and moving averages
        // ----------------------------------------------------------------------------

        // Moving Averages and Crosses
        let mut ema_3 = MA::EMA(3).init(c0.close).expect("Failed to initialize EMA 3");
        let mut ema_3_stdev = StDevVarianceSqrt::new(3, &0.0).expect("Failed to initialize EMA 3 StDev");

        let mut ema_5 = MA::EMA(5).init(c0.close).expect("Failed to initialize EMA 5");
        let mut ema_5_stdev = StDevVarianceSqrt::new(5, &0.0).expect("Failed to initialize EMA 5 StDev");

        let mut ema_8 = MA::EMA(8).init(c0.close).expect("Failed to initialize EMA 8");
        let mut ema_8_stdev = StDevVarianceSqrt::new(8, &0.0).expect("Failed to initialize EMA 8 StDev");

        let mut ema_13 = MA::EMA(13).init(c0.close).expect("Failed to initialize EMA 13");
        let mut ema_13_stdev = StDevVarianceSqrt::new(13, &0.0).expect("Failed to initialize EMA 13 StDev");

        let mut ema_21 = MA::EMA(21).init(c0.close).expect("Failed to initialize EMA 21");
        let mut ema_21_stdev = StDevVarianceSqrt::new(21, &0.0).expect("Failed to initialize EMA 21 StDev");

        let mut ema_21 = MA::EMA(21).init(c0.close).expect("Failed to initialize EMA 21");

        let mut ema_3_cross = Cross::new((), &(c1.close, ema_3.next(&c1.close))).expect("Failed to initialize EMA 3 Cross");
        let mut ema_5_cross = Cross::new((), &(c1.close, ema_5.next(&c1.close))).expect("Failed to initialize EMA 5 Cross");
        let mut ema_8_cross = Cross::new((), &(c1.close, ema_8.next(&c1.close))).expect("Failed to initialize EMA 8 Cross");
        let mut ema_13_cross = Cross::new((), &(c1.close, ema_13.next(&c1.close))).expect("Failed to initialize EMA 13 Cross");
        let mut ema_21_cross = Cross::new((), &(c1.close, ema_21.next(&c1.close))).expect("Failed to initialize EMA 21 Cross");

        let mut ema_3_5_cross = Cross::new((), &(ema_3.peek_next(&c1.close), ema_5.peek_next(&c1.close))).expect("Failed to initialize EMA 3 5 Cross");
        let mut ema_5_8_cross = Cross::new((), &(ema_5.peek_next(&c1.close), ema_8.peek_next(&c1.close))).expect("Failed to initialize EMA 5 8 Cross");
        let mut ema_8_13_cross = Cross::new((), &(ema_8.peek_next(&c1.close), ema_13.peek_next(&c1.close))).expect("Failed to initialize EMA 8 13 Cross");
        let mut ema_13_21_cross = Cross::new((), &(ema_13.peek_next(&c1.close), ema_21.peek_next(&c1.close))).expect("Failed to initialize EMA 13 21 Cross");

        // Parabolic SARs
        let mut psar_scalp = ParabolicSAR { af_step: 0.02, af_max: 0.1 }
            .init(&c0)
            .expect("Failed to initialize Parabolic SAR (Scalping)");

        let mut psar_trend = ParabolicSAR { af_step: 0.01, af_max: 0.2 }
            .init(&c0)
            .expect("Failed to initialize Parabolic SAR (Trend-Following)");

        // Linear Volatility
        let mut linvol_5_close = LinearVolatility::new(5, &c0.close).expect("Failed to initialize Linear Volatility 5 Close");

        // Initializing the RSI 3 instance
        let mut rsi_3 = RelativeStrengthIndex {
            ma:            MA::RMA(3),
            smooth_ma:     MA::WMA(3),
            primary_zone:  0.001,
            smoothed_zone: 0.01,
            source:        Source::Close,
        }
        .init(&c0)
        .expect("Failed to initialize RSI 3");

        // Initializing the RSI 14 instance
        let mut rsi_14 = RelativeStrengthIndex {
            ma:            MA::RMA(14),
            smooth_ma:     MA::WMA(14),
            primary_zone:  0.3,
            smoothed_zone: 0.2,
            source:        Source::Close,
        }
        .init(&c0)
        .expect("Failed to initialize RSI 14");

        // True Range
        let mut tr = TR::new(&c0).expect("Failed to initialize TR");
        let mut tr_roc = RateOfChange::new(1, &tr.next(&c0)).expect("Failed to initialize TR ROC");

        // Standard Deviation of True Range
        let mut stdev_tr_ln = StDevVarianceSqrt::new(14, &tr.method_peek_next(&c1).ln()).expect("Failed to initialize StDev LOG(TR)");
        let mut stdev_tr_roc = StDevVarianceSqrt::new(14, &tr.next(&c1)).expect("Failed to initialize StDev TR ROC");
        let mut stdev_high_low = StDevVarianceSqrt::new(14, &(c0.high - c0.low)).expect("Failed to initialize StDev High Low");
        let mut stdev_open_close = StDevVarianceSqrt::new(14, &(c0.open - c0.close)).expect("Failed to initialize StDev Open Close");

        // Bollinger Bands
        let mut bb = BollingerBands {
            avg_size: 20,
            source:   Source::Close,
            sigma:    2.0,
        }
        .init(&c0)
        .expect("Failed to initialize Bollinger Bands");

        // Keltner Channel
        let mut kc = KeltnerChannel {
            ma:     MA::EMA(20),
            sigma:  1.0,
            source: Source::Close,
        }
        .init(&c0)
        .expect("Failed to initialize Keltner Channel");

        // Kaufman Adaptive Moving Average
        let mut kama = KAMA {
            period1:       21,
            period2:       2,
            period3:       30,
            filter_period: 21,
            square_smooth: true,
            k:             0.3,
            source:        Source::Close,
        }
        .init(&c0)
        .expect("Failed to initialize KAMA");

        // MACD
        let mut macd = MACD {
            ma1:    MA::EMA(5),
            ma2:    MA::EMA(13),
            signal: MA::EMA(4),
            source: Source::Close,
        }
        .init(&c0)
        .expect("Failed to initialize MACD");

        // Know Sure Thing
        let mut kst = KnowSureThing {
            period1: 10,
            period2: 15,
            period3: 20,
            period4: 30,
            ma1:     MA::SMA(10),
            ma2:     MA::SMA(10),
            ma3:     MA::SMA(10),
            ma4:     MA::SMA(15),
            signal:  MA::SMA(9),
        }
        .init(&c0)
        .expect("Failed to initialize KST");

        // The working state is used to store the current state of the indicators and moving averages
        self.working_state = Some(WorkingState {
            i: IndicatorState {
                ema_3: ema_3.clone(),
                ema_3_stdev: ema_3_stdev.clone(),
                ema_5: ema_5.clone(),
                ema_5_stdev: ema_5_stdev.clone(),
                ema_8: ema_8.clone(),
                ema_8_stdev: ema_8_stdev.clone(),
                ema_13: ema_13.clone(),
                ema_13_stdev: ema_13_stdev.clone(),
                ema_21: ema_21.clone(),
                ema_21_stdev: ema_21_stdev.clone(),
                rsi_3: rsi_3,
                rsi_14: rsi_14,
                bb: bb.clone(),
                kc: kc.clone(),
                kama: kama.clone(),
                macd: macd.clone(),
                kst,
                tr: tr,
                tr_roc: tr_roc,
                psar_scalp: psar_scalp.clone(),
                psar_trend: psar_trend.clone(),
                stdev_tr_ln,
                stdev_tr_roc: stdev_tr_roc,
                stdev_high_low: stdev_high_low,
                stdev_open_close: stdev_open_close,
            },

            // IMPORTANT: Further only the moving averages that are already initialized in the working state will be computed
            p:  PersistentIndicatorState {
                ema_21: ema_21,
                ema_21_cross: ema_21_cross,
                bb: bb,
                kc: kc,
                kama: kama,
                macd: macd,
                psar_scalp,
                psar_trend,
                linvol_5_close: linvol_5_close,
            },
            ma: vec![],
        });

        let ws = self.working_state.as_mut().expect("Failed to get the working state");

        // ----------------------------------------------------------------------------
        // The first state is used to store the first computed state of the indicators and moving averages.
        // WARNING: Shadowing the instance variables (e.g. rsi_14, rsi_14_wma_14) to store the first values.
        // ----------------------------------------------------------------------------

        // let rsi_14 = ws.indicator.rsi_14.next(&c1);
        // let rsi_14_wma_14 = ws.indicator.rsi_14_wma_14.next(&rsi_14.value(0));

        // ----------------------------------------------------------------------------
        // Initialize the Moving Averages
        // ----------------------------------------------------------------------------

        // A vector to store the first computed moving averages of the second candle
        // let mut mas = vec![];

        // Initialize the instances for the Fibonacci sequence of periods
        for period in FIB_SEQUENCE.into_iter() {
            if period > max_period {
                break;
            }

            ws.ma
                .push((MaType::EMA, period, MA::EMA(period as PeriodType).init(c0.close).expect("Failed to initialize EMA")));

            /*
            ws.ma
                .push((MaType::WSMA, period, MA::WSMA(period as PeriodType).init(c0.close).expect("Failed to initialize WWMA")));

            ws.ma
                .push((MaType::WMA, period, MA::WMA(period as PeriodType).init(c0.close).expect("Failed to initialize WMA")));
             */
        }

        Ok(())
    }

    /// The head is the newest candle in the series, which is the working candle if it exists or the last finalized candle.
    #[inline]
    pub fn head_candle(&self) -> Option<&Candle> {
        self.working_candle.as_ref().or(self.candles.head())
    }

    #[inline]
    pub fn head_candle_mut(&mut self) -> Option<&mut Candle> {
        self.working_candle.as_mut().or(self.candles.head_mut())
    }

    #[inline]
    pub fn tail_candle(&self) -> Option<&Candle> {
        self.candles.tail()
    }

    #[inline]
    pub fn head_state(&self) -> Option<&ComputedState> {
        self.states.head()
    }

    #[inline]
    pub fn head_state_mut(&mut self) -> Option<&mut ComputedState> {
        self.states.head_mut()
    }

    #[inline]
    pub fn tail_state(&self) -> Option<&ComputedState> {
        self.states.tail()
    }

    /// The working candle is not included in the series until it is finalized.
    #[inline]
    pub fn last_finalized_candle(&self) -> Option<&Candle> {
        self.candles.head()
    }

    /// Unlike the series' candles, the computed states are stored in the series, as a head element.
    /// The head state is always a working version, therefore the last finalized state is the second state.
    #[inline]
    pub fn last_finalized_state(&self) -> Option<&ComputedState> {
        self.states.get(1)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.candles.is_empty()
    }

    /// Returns the number of candles in the series (excluding the working candle).
    #[inline]
    pub fn num_candles(&self) -> usize {
        self.candles.len() + if self.working_candle.is_some() { 1 } else { 0 }
    }

    #[inline]
    pub fn num_states(&self) -> usize {
        self.states.len()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.candles.capacity()
    }

    #[inline]
    pub fn symbol(&self) -> &Ustr {
        &self.symbol
    }

    #[inline]
    pub fn timeframe(&self) -> Timeframe {
        self.timeframe
    }

    #[inline]
    pub fn base_timeframe(&self) -> Timeframe {
        self.base_timeframe
    }

    #[inline]
    pub fn fill_gaps(&mut self, target_candle: &Candle) -> Result<(), CandleSeriesError> {
        // If the series is empty or is less than 1 candle, there's nothing to do.
        if self.is_empty() {
            return Ok(());
        }

        loop {
            let Some(last_candle) = self.candles.head().cloned() else {
                warn!("No candles in the series, skipping filling gaps");
                return Ok(());
            };

            if target_candle.open_time > last_candle.close_time {
                self.add_filler_candle(&last_candle)?;
            } else {
                break;
            }
        }

        Ok(())
    }

    /// Returns a reference to a candle by reverse index.
    pub fn get_candle(&self, index: usize) -> Option<&Candle> {
        if index == 0 {
            return self.working_candle.as_ref();
        }

        self.candles.get(index)
    }

    /// Returns a mutable reference to a candle by reverse index.
    pub fn get_candle_mut(&mut self, index: usize) -> Option<&mut Candle> {
        if index == 0 {
            return self.working_candle.as_mut();
        }

        self.candles.get_mut(index)
    }

    pub fn get_state(&self, index: usize) -> Option<&ComputedState> {
        self.states.get(index)
    }

    // FIXME: Call `compute` after pushing the working candle
    #[inline]
    pub fn push_candle(&mut self, new_candle: Candle) -> Result<Option<ComputedState>, CandleSeriesError> {
        // First, validate the incoming candle to ensure it matches the series' expectations
        if new_candle.symbol != self.symbol {
            return Err(CandleSeriesError::InvalidSymbol(new_candle.symbol, self.symbol));
        }

        if new_candle.timeframe != self.timeframe && new_candle.timeframe != self.base_timeframe {
            return Err(CandleSeriesError::InvalidTimeframe {
                given:    new_candle.timeframe,
                expected: self.timeframe,
            });
        }

        // Taking the backed up candle if it exists, otherwise using the new candle
        let new_candle = self.next_candle_backup.replace(new_candle).unwrap_or_else(|| new_candle);

        if let Some(last_finalized_candle) = self.last_finalized_candle() {
            if new_candle.open_time < last_finalized_candle.close_time {
                return Ok(None);
            }
        }

        // FIXME
        // if self.fill_gaps {
        //     self.fill_gaps(&new_candle)?;
        // }

        /*
        // FIXME: Maybe it makes sense to allow full candles to push historical data but use only trades for real-time data
        // If the series is trade-only, we do not process full candles
        if self.trades_only {
            error!("Trade-only series cannot process candles");
            return Err(CandleSeriesError::ComputeTradeOnlySeries);
        }
         */

        // If the incoming candle's timeframe matches the series' base timeframe, process it accordingly
        if self.timeframe != self.base_timeframe && new_candle.timeframe == self.base_timeframe {
            // The working candle is the actual state of the main timeframe candle.
            let working_candle = self.working_candle.get_or_insert_with(|| {
                let open_time = new_candle.open_time;
                let close_time = open_time + self.timeframe.duration() - self.timeframe.close_time_offset();

                // NOTE: Explicitly setting the working index to 0 to avoid any potential issues and confusion.
                self.working_index = 0;

                let candle = Candle {
                    symbol: self.symbol,
                    timeframe: self.timeframe, // The timeframe of the series
                    open_time,
                    close_time,
                    is_final: false,
                    ..new_candle // The new candle's data is used as the initial state of the working candle
                };

                candle
            });

            // The snapshot is storing the aggregated data of base timeframe candles until the goal is met.
            let backup = self.working_candle_backup.get_or_insert(working_candle.clone());

            // Initialization or update of working_candle and working_candle_backup
            if new_candle.is_final && new_candle.open_time == working_candle.open_time {
                // Updating the backup with the new finalized candle's data
                if self.working_index == 0 {
                    backup.high = backup.high.max(new_candle.high);
                    backup.low = backup.low.min(new_candle.low);
                    backup.close = new_candle.close;
                    backup.volume = new_candle.volume;
                    backup.number_of_trades = new_candle.number_of_trades;
                    backup.quote_asset_volume = new_candle.quote_asset_volume;
                    backup.taker_buy_quote_asset_volume = new_candle.taker_buy_quote_asset_volume;
                    backup.taker_buy_base_asset_volume = new_candle.taker_buy_base_asset_volume;
                } else {
                    backup.high = backup.high.max(new_candle.high);
                    backup.low = backup.low.min(new_candle.low);
                    backup.close = new_candle.close;
                    backup.volume += new_candle.volume;
                    backup.number_of_trades += new_candle.number_of_trades;
                    backup.quote_asset_volume += new_candle.quote_asset_volume;
                    backup.taker_buy_quote_asset_volume += new_candle.taker_buy_quote_asset_volume;
                    backup.taker_buy_base_asset_volume += new_candle.taker_buy_base_asset_volume;
                }

                // Copying the backup to the working candle
                *working_candle = backup.clone();

                // Increment the working index towards the goal.
                self.working_index += 1;
            } else {
                // Handle aggregation of base timeframe candle into the working candle.
                // Always update working_candle with data combined from backup and new_candle
                if self.working_index == 0 {
                    working_candle.high = working_candle.high.max(new_candle.high);
                    working_candle.low = working_candle.low.min(new_candle.low);
                    working_candle.close = new_candle.close;
                    working_candle.volume = new_candle.volume;
                    working_candle.number_of_trades = new_candle.number_of_trades;
                    working_candle.quote_asset_volume = new_candle.quote_asset_volume;
                    working_candle.taker_buy_quote_asset_volume = new_candle.taker_buy_quote_asset_volume;
                    working_candle.taker_buy_base_asset_volume = new_candle.taker_buy_base_asset_volume;
                } else {
                    working_candle.high = backup.high.max(new_candle.high);
                    working_candle.low = backup.low.min(new_candle.low);
                    working_candle.close = new_candle.close;
                    working_candle.volume = backup.volume + new_candle.volume;
                    working_candle.number_of_trades = backup.number_of_trades + new_candle.number_of_trades;
                    working_candle.quote_asset_volume = backup.quote_asset_volume + new_candle.quote_asset_volume;
                    working_candle.taker_buy_quote_asset_volume = backup.taker_buy_quote_asset_volume + new_candle.taker_buy_quote_asset_volume;
                    working_candle.taker_buy_base_asset_volume = backup.taker_buy_base_asset_volume + new_candle.taker_buy_base_asset_volume;
                }
            }

            // Check if the aggregation goal is met.
            if self.working_index >= self.working_index_goal && new_candle.is_final {
                // Finalize the working candle and add it to the series.
                working_candle.is_final = true;

                // Push the working candle into the series.
                self.candles.push(std::mem::replace(&mut self.working_candle, None).unwrap());

                // Reset for the next aggregation cycle.
                self.working_candle = None;
                self.working_candle_backup = None;
                self.working_index = 0;
            }

            println!(
                "Base timeframe: {:?} Series Timeframe: {:?} Working Index: {} Working Index Goal: {}",
                self.base_timeframe, self.timeframe, self.working_index, self.working_index_goal
            );

            return self.compute();
        } else {
            // Ensure the candle matches the series' timeframe
            if new_candle.timeframe != self.timeframe {
                debug!("Invalid timeframe: given={:?}, expected={:?}", new_candle.timeframe, self.timeframe);

                return Err(CandleSeriesError::InvalidTimeframe {
                    given:    new_candle.timeframe,
                    expected: self.timeframe,
                });
            }

            /*
            // Check if there's a working candle. If not, set the new candle as the working candle.
            if self.working_candle.is_none() && !new_candle.is_final {
                self.working_candle = Some(new_candle);
                return self.compute();
            }
             */

            if self.working_candle.is_none() {
                return if new_candle.is_final {
                    self.candles.push(new_candle);
                    self.compute()
                } else {
                    self.working_candle = Some(new_candle);
                    self.compute()
                };
            }

            // Finalizing the working candle if it's time to do so
            // NOTE: Arguing with the compiler
            if let Some(working_candle) = self.working_candle.as_ref().cloned() {
                if new_candle.open_time > working_candle.close_time && working_candle.is_final {
                    self.candles.push(working_candle);
                    self.working_candle = None; // Prepare for the next candle

                    // Remember the new candle for the next iteration
                    self.next_candle_backup = Some(new_candle);

                    // IMPORTANT: Compute the series after pushing the working candle
                    return self.compute();
                }
            }

            // Flag to clear the working candle after pushing it into the series
            let mut clear_working_candle = false;

            // If there is a working candle, decide whether to integrate the new candle into it,
            // finalize the working candle, or replace the working candle with the new one.
            if let Some(working_candle) = &mut self.working_candle {
                // An explicit check to prevent pushing a new candle that overlaps with the
                // current working candle's timeframe or duplicates an existing candle's open_time
                if new_candle.open_time <= working_candle.open_time {
                    return Ok(None);
                }

                // If the new candle's period is contiguous with the working candle, we might want to merge or update it.
                if new_candle.open_time == working_candle.open_time && new_candle.close_time == working_candle.close_time {
                    // Merge the new candle's data into the working candle.
                    working_candle.open = new_candle.open;
                    working_candle.high = working_candle.high.max(new_candle.high);
                    working_candle.low = working_candle.low.min(new_candle.low);
                    working_candle.close = new_candle.close;
                    working_candle.volume = new_candle.volume;
                    working_candle.number_of_trades = new_candle.number_of_trades;
                    working_candle.quote_asset_volume = new_candle.quote_asset_volume;
                    working_candle.taker_buy_quote_asset_volume = new_candle.taker_buy_quote_asset_volume;
                    working_candle.taker_buy_base_asset_volume = new_candle.taker_buy_base_asset_volume;

                    if !working_candle.is_final && new_candle.is_final {
                        working_candle.is_final = true;
                    }

                    // If the new candle is final, finalize the working candle and push it into the series.
                    // NOTE: Just a precaution to ensure that final flag is not reset back to false
                    if working_candle.is_final {
                        self.candles.push(working_candle.clone());
                        // self.compute()?;
                        clear_working_candle = true;
                    }
                } else {
                    if new_candle.open_time > working_candle.close_time {
                        working_candle.is_final = true;
                        self.candles.push(working_candle.clone());
                        // self.compute()?;
                        clear_working_candle = true;
                    } else {
                        self.candles.push(self.working_candle.take().unwrap());
                        // self.compute()?;
                        self.working_candle = Some(new_candle);
                        self.working_candle_backup = None; // Just in case
                    }
                }
            }

            // Clear the working candle to prepare for the next one.
            if clear_working_candle {
                self.working_candle = None; // Prepare for the next candle
            }
        }

        self.compute()
    }

    /// Function to add a filler candle based on the last candle's close price
    /// NOTE: The filler candle is used only to fill gaps in the past and must not take place of the working candle.
    #[inline]
    fn add_filler_candle(&mut self, last_candle: &Candle) -> Result<(), CandleSeriesError> {
        let new_open_time = last_candle.open_time + last_candle.timeframe.duration();
        let new_close_time = new_open_time + last_candle.timeframe.duration() - last_candle.timeframe.close_time_offset();

        let filler_candle = Candle {
            symbol:                       self.symbol,
            timeframe:                    self.timeframe,
            open_time:                    new_open_time,
            close_time:                   new_close_time,
            open:                         last_candle.close,
            high:                         last_candle.close,
            low:                          last_candle.close,
            close:                        last_candle.close,
            volume:                       0.0,
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     false,
        };

        eprintln!("filler_candle = {:#?}", filler_candle);

        // Push the filler candle into the series.
        self.candles.push(filler_candle);

        Ok(())
    }

    /// Updates the series with a new trade, affecting the state of the current working candle.
    #[inline]
    pub fn apply_trade(&mut self, trade: ExchangeTradeEvent) -> Result<Option<ComputedState>, CandleSeriesError> {
        if trade.symbol != self.symbol {
            return Err(CandleSeriesError::InvalidSymbol(trade.symbol, self.symbol));
        }

        if let Some(working_candle) = self.working_candle.as_mut() {
            if trade.trade_time <= working_candle.close_time {
                // Updating the working candle with trade data
                working_candle.high = working_candle.high.max(trade.price.0);
                working_candle.low = working_candle.low.min(trade.price.0);
                working_candle.close = trade.price.0;
                working_candle.volume += trade.quantity.0;
                working_candle.number_of_trades += 1;
                working_candle.quote_asset_volume += trade.price.0 * trade.quantity.0;
                working_candle.taker_buy_quote_asset_volume += if trade.is_buyer_maker { 0.0 } else { trade.price.0 * trade.quantity.0 };
                working_candle.taker_buy_base_asset_volume += if trade.is_buyer_maker { trade.quantity.0 } else { 0.0 };
            } else {
                // Handle the trade that belongs to a future timeframe
                if let Some(working_candle) = &mut self.working_candle {
                    working_candle.is_final = true;
                    self.candles.push(std::mem::replace(&mut self.working_candle, None).unwrap());

                    // Compute the series after pushing the working candle
                    return self.compute(); // FIXME: Decide whether I can afford to return the result here
                }

                let next_open_time = calculate_candle_start_time(trade.event_time, self.base_timeframe.duration().num_milliseconds());
                // let next_open_time = working_candle.open_time + self.base_timeframe.duration();
                let next_close_time = next_open_time + self.base_timeframe.duration() - self.base_timeframe.close_time_offset();

                self.working_candle = Some(Candle {
                    symbol:                       self.symbol,
                    timeframe:                    self.base_timeframe,
                    open_time:                    next_open_time,
                    close_time:                   next_close_time,
                    is_final:                     false,
                    open:                         trade.price.0,
                    high:                         trade.price.0,
                    low:                          trade.price.0,
                    close:                        trade.price.0,
                    volume:                       trade.quantity.0,
                    number_of_trades:             1,
                    quote_asset_volume:           trade.price.0 * trade.quantity.0,
                    taker_buy_quote_asset_volume: if trade.is_buyer_maker { 0.0 } else { trade.price.0 * trade.quantity.0 },
                    taker_buy_base_asset_volume:  if trade.is_buyer_maker { trade.quantity.0 } else { 0.0 },
                });
            }
        } else {
            let next_open_time = calculate_candle_start_time(trade.event_time, self.base_timeframe.duration().num_milliseconds());
            // let next_open_time = working_candle.open_time + self.base_timeframe.duration();
            let next_close_time = next_open_time + self.base_timeframe.duration() - self.base_timeframe.close_time_offset();

            self.working_candle = Some(Candle {
                symbol:                       self.symbol,
                timeframe:                    self.base_timeframe,
                open_time:                    next_open_time,
                close_time:                   next_close_time,
                is_final:                     false,
                open:                         trade.price.0,
                high:                         trade.price.0,
                low:                          trade.price.0,
                close:                        trade.price.0,
                volume:                       trade.quantity.0,
                number_of_trades:             1,
                quote_asset_volume:           trade.price.0 * trade.quantity.0,
                taker_buy_quote_asset_volume: if trade.is_buyer_maker { 0.0 } else { trade.price.0 * trade.quantity.0 },
                taker_buy_base_asset_volume:  if trade.is_buyer_maker { trade.quantity.0 } else { 0.0 },
            });
        }

        self.compute()
    }

    /// Returns an iterator over all candles including the working candle at the start.
    /// The working candle is the first item in the iterator.
    pub fn iter_candles_front_to_back(&self) -> impl Iterator<Item = &Candle> + '_ {
        self.working_candle.iter().chain(self.candles.iter())
    }

    /// Returns an iterator over all candles including the working candle at the end.
    /// The working candle is the last item in the iterator.
    pub fn iter_candles_back_to_front(&self) -> impl Iterator<Item = &Candle> + '_ {
        self.candles.iter_rev().chain(self.working_candle.iter())
    }

    /// Returns an iterator over all computed states including the working state at the start.
    /// NOTE: The working state is the first item in the iterator.
    pub fn iter_states_front_to_back(&self) -> impl Iterator<Item = &ComputedState> + '_ {
        self.states.iter()
    }

    /// Returns an iterator over all computed states including the working state at the end.
    /// NOTE: The working state is the last item in the iterator.
    pub fn iter_states_back_to_front(&self) -> impl Iterator<Item = &ComputedState> + '_ {
        self.states.iter_rev()
    }

    // TODO: react stronger to big RSI spikes in momentum over a short period of time
    /// Computes the indicators and moving averages for the series using the latest data.
    pub fn compute(&mut self) -> Result<Option<ComputedState>, CandleSeriesError> {
        let start = Instant::now();
        let num_candles = self.num_candles();

        // There are not enough candles to compute the indicators and moving averages.
        if self.num_candles() < 2 {
            eprintln!("{:?}  {} {}", self.timeframe, self.num_candles(), self.candles.len());
            warn!("Not enough candles to compute indicators and moving averages");
            return Ok(None);
        }

        /*
        // The working candle is the actual state of the main series' timeframe candle.
        let Some(wc) = self.working_candle else {
            debug!("{}: {} > No working candle to compute indicators and moving averages, skipping computation", self.symbol, self.timeframe.to_string());
            return Ok(None);
        };
         */

        let Some(hc) = self.head_candle().cloned() else {
            debug!("{}: {} > No head candle to compute indicators and moving averages, skipping computation", self.symbol, self.timeframe.to_string());
            return Ok(None);
        };

        /*
        // FIXME: If the last finalized candle doesn't have a computed state, we need to compute it before proceeding.
        {
            if let Some(lfc) = self.last_finalized_candle() {
                if let Some(lfs) = self.last_finalized_state() {
                    if lfc.open_time != lfs.candle.open_time {
                        self.compute()?;
                    }
                }
            }
        }
         */

        // The working state is used to store the current state of the indicators and moving averages
        let ws = match self.working_state {
            Some(ref mut s) => s,
            None => match self.initialize_working_state() {
                Ok(()) => self.working_state.as_mut().unwrap(),
                Err(CandleSeriesError::NotEnoughCandles) => {
                    return Ok(None);
                }
                Err(e) => {
                    return Err(e);
                }
            },
        };

        /*
        if hc.is_final && self.is_live && self.timeframe == Timeframe::S1 {
            println!("{:#?}", hc);
        }
         */

        // ----------------------------------------------------------------------------
        // IMPORTANT: The computed state is separate from the persistent indicator states
        // ----------------------------------------------------------------------------

        // The computed state is used to store the computed values of the indicators and moving averages
        let mut cs = ComputedState {
            candle:                          hc.clone(),
            i:                               ComputedIndicatorState {
                candle:           hc.clone(),
                ema_3:            0.0,
                ema_3_stdev:      0.0,
                ema_5:            0.0,
                ema_5_stddev:     0.0,
                ema_8:            0.0,
                ema_8_stddev:     0.0,
                ema_13:           0.0,
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
                ema_13_stddev:    0.0,
                psar_trend:       Default::default(),
            },
            p:                               Default::default(),
            ma:                              Vec::with_capacity(ws.ma.len()),
            indicator_computation_time:      Default::default(),
            moving_average_computation_time: Default::default(),
            total_computation_time:          Default::default(),
        };

        // ----------------------------------------------------------------------------
        // WARNING: Non-Persistent Computations
        // ----------------------------------------------------------------------------

        // Computing the indicators for the current candle
        let indicator_start = Instant::now();
        if hc.is_final {
            let tr = ws.i.tr.next(&hc);

            cs.i.tr = ws.i.tr.next(&hc);
            cs.i.rsi_3 = ws.i.rsi_3.next(&hc);
            cs.i.rsi_14 = ws.i.rsi_14.next(&hc);
            cs.i.ema_3 = ws.i.ema_3.next(&hc.close);
            cs.i.ema_3_stdev = ws.i.ema_3_stdev.next(&cs.i.ema_3);
            cs.i.ema_21 = ws.i.ema_21.next(&hc.close);
            cs.i.stdev_tr_ln = ws.i.stdev_tr_ln.next(&tr.ln());
            cs.i.stdev_tr_roc = ws.i.stdev_tr_roc.next(&tr);
            cs.i.stdev_high_low = ws.i.stdev_high_low.next(&(hc.high - hc.low));
            cs.i.stdev_open_close = ws.i.stdev_open_close.next(&(hc.open - hc.close));
            cs.i.kc = ws.i.kc.next(&hc);
            cs.i.bb = ws.i.bb.next(&hc);
            cs.i.kama = ws.i.kama.next(&hc);
        } else {
            let tr = ws.i.tr.method_peek_next(&hc);

            cs.i.rsi_3 = ws.i.rsi_3.peek_next(&hc);
            cs.i.rsi_3 = ws.i.rsi_3.peek_next(&hc);
            cs.i.rsi_14 = ws.i.rsi_14.peek_next(&hc);
            cs.i.ema_3 = ws.i.ema_3.peek_next(&hc.close);
            cs.i.ema_3_stdev = ws.i.ema_3_stdev.method_peek_next(&cs.i.ema_3);
            cs.i.ema_21 = ws.i.ema_21.peek_next(&hc.close);
            cs.i.stdev_tr_ln = ws.i.stdev_tr_ln.method_peek_next(&tr.ln());
            cs.i.stdev_tr_roc = ws.i.stdev_tr_roc.method_peek_next(&tr);
            cs.i.stdev_high_low = ws.i.stdev_high_low.method_peek_next(&(hc.high - hc.low));
            cs.i.stdev_open_close = ws.i.stdev_open_close.method_peek_next(&(hc.open - hc.close));
            cs.i.kc = ws.i.kc.peek_next(&hc);
            cs.i.bb = ws.i.bb.peek_next(&hc);
            cs.i.kama = ws.i.kama.peek_next(&hc);
        }

        // ----------------------------------------------------------------------------
        // WARNING: Persistent Computations
        // ----------------------------------------------------------------------------

        if hc.is_final {
            cs.p.ema_21 = ws.p.ema_21.next(&hc.close);
            cs.p.ema_21_cross = ws.p.ema_21_cross.next(&(hc.close, cs.p.ema_21));
            cs.p.kc = ws.p.kc.next(&hc);
            cs.p.bb = ws.p.bb.next(&hc);
            cs.p.kama = ws.p.kama.next(&hc);
        }

        // Storing the computation time for the indicators
        cs.indicator_computation_time = indicator_start.elapsed();

        let ma_start = Instant::now();
        // Compute the moving averages for the current candle
        for ma in ws.ma.iter_mut() {
            /*
            if period as usize > num_candles {
                break;
            }
             */

            if hc.is_final {
                cs.ma.push((ma.0, ma.1, f!(ma.2.next(&hc.close))));
            } else {
                cs.ma.push((ma.0, ma.1, f!(ma.2.peek_next(&hc.close))));
            }
        }
        cs.moving_average_computation_time = ma_start.elapsed();

        // Time to compute the moving averages
        cs.total_computation_time += start.elapsed();

        // If the working candle is finalized, push it into the series, otherwise update the head state.
        if hc.is_final {
            self.states.push(cs.clone());
        } else {
            if self.states.len() > 0 {
                *self.states.head_mut().unwrap() = cs.clone();
            } else {
                self.states.push(cs.clone());
            }
        }

        Ok(Some(cs))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;
    use itertools::Itertools;
    use yata::core::ValueType;

    use crate::{
        config::Config,
        database::{Database, DatabaseBackend},
        f,
        util::{remove_seconds, truncate_to_midnight},
    };

    use super::*;

    #[test]
    fn test_candle_series() {
        let timeframe = Timeframe::H1;
        let num_candles = 10;
        let buffer_capacity = 5;

        let mut series = CandleSeries::new(Ustr::from("BTCUSDT"), timeframe, None, buffer_capacity, 89, true, false, false);
        let mut history = Vec::new();

        // Push initial candles to populate the series
        let initial_open_time = Utc::now();
        let mut last_candle = Candle {
            symbol:                       Ustr::from("BTCUSDT"),
            timeframe:                    timeframe,
            open_time:                    initial_open_time,
            close_time:                   initial_open_time + Duration::hours(1),
            open:                         100.0,
            high:                         105.0,
            low:                          95.0,
            close:                        100.0,
            volume:                       1000.0,
            number_of_trades:             100,
            quote_asset_volume:           102000.0,
            taker_buy_quote_asset_volume: 51000.0,
            taker_buy_base_asset_volume:  500.0,
            is_final:                     true,
        };

        series.push_candle(last_candle.clone()).unwrap();

        // Populate with dummy candles
        for _ in 1..num_candles {
            let new_open_time = last_candle.open_time + Duration::hours(1);
            let new_candle = Candle {
                open_time: new_open_time,
                close_time: new_open_time + Duration::hours(1),
                close: last_candle.close + 1.0,
                ..last_candle.clone()
            };

            series.push_candle(new_candle.clone()).unwrap();

            if new_candle.is_final {
                history.push(new_candle.clone());
            }

            last_candle = new_candle;
        }

        // Now push a non-final "working" candle
        let working_candle = Candle {
            is_final: false,
            close: last_candle.close + 1.0,
            ..last_candle.clone()
        };

        series.push_candle(working_candle).unwrap();

        // Verify the working candle is set correctly
        assert_eq!(series.working_candle.as_ref().unwrap().close, last_candle.close + 1.0);
        assert_eq!(series.working_candle.as_ref().unwrap().is_final, false);

        // Update the "working" candle
        let updated_working_candle = Candle {
            high: 110.0,
            low: 90.0,
            close: last_candle.close + 2.0,
            volume: 2000.0,
            is_final: true,
            ..last_candle
        };

        series.push_candle(updated_working_candle.clone()).unwrap();

        // Verify the update
        let last_finalized_candle = series.last_finalized_candle().expect("Expected a last finalized candle");
        assert_eq!(last_finalized_candle.high, 110.0, "High should be updated.");
        assert_eq!(last_finalized_candle.low, 90.0, "Low should be updated.");
        assert_eq!(last_finalized_candle.close, last_candle.close + 2.0, "Close should be updated.");
        assert_eq!(last_finalized_candle.volume, 2000.0, "Volume should be updated.");
        assert!(last_finalized_candle.is_final, "The candle should be marked as final.");

        assert_eq!(series.num_candles(), buffer_capacity);
        assert_eq!(series.capacity(), buffer_capacity);
        assert_eq!(series.tail_candle(), Some(&history[num_candles - buffer_capacity - 1]));
        assert_eq!(series.last_finalized_candle(), Some(&updated_working_candle));
        assert_eq!(series.head_candle(), None);

        // Ensure the working candle is now cleared since the last one was final
        assert!(series.working_candle.is_none(), "The working candle should be cleared after pushing a final candle.");
    }

    #[test]
    fn test_iter_back_to_front() {
        use super::*;
        use crate::{candle::Candle, timeframe::Timeframe};

        let timeframe = Timeframe::H1;
        let num_candles = 10;
        let buffer_capacity = 5;

        let mut series = CandleSeries::new(Ustr::from("BTCUSDT"), Timeframe::H1, None, buffer_capacity, 89, true, false, false);

        // The initial candle that the series will be "pushing away" from
        let initial_open_time = remove_seconds(Utc::now());
        let initial_candle = Candle {
            symbol:                       Ustr::from("BTCUSDT"),
            timeframe:                    Timeframe::H1,
            open_time:                    initial_open_time,
            close_time:                   initial_open_time + timeframe.duration() - timeframe.close_time_offset(),
            open:                         100.0,
            high:                         100.0,
            low:                          100.0,
            close:                        100.0,
            volume:                       100.0,
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     true,
        };

        // Pushing the initial candle
        series.push_candle(initial_candle).unwrap();

        // The last candle that was pushed into the series
        let mut last_candle = initial_candle;
        let mut history = vec![last_candle.clone()];

        // Populate with dummy candles
        for i in 0..num_candles {
            let new_open_time = last_candle.open_time + timeframe.duration();
            let new_close_time = new_open_time + last_candle.timeframe.duration() - last_candle.timeframe.close_time_offset();

            let new_candle = Candle {
                symbol:                       last_candle.symbol,
                timeframe:                    last_candle.timeframe,
                open_time:                    new_open_time,
                close_time:                   new_close_time,
                open:                         last_candle.open,
                high:                         last_candle.high,
                low:                          last_candle.low,
                close:                        last_candle.close + 1.0,
                volume:                       100.0,
                number_of_trades:             0,
                quote_asset_volume:           0.0,
                taker_buy_quote_asset_volume: 0.0,
                taker_buy_base_asset_volume:  0.0,
                is_final:                     !(i == num_candles - 1),
            };

            eprintln!("{} -> new_candle = {:#?}", i, new_candle);

            series.push_candle(new_candle).unwrap();

            if new_candle.is_final {
                history.push(new_candle.clone());
            }

            last_candle = new_candle;
        }

        let mut iter = series.iter_candles_back_to_front();
        let iter_items = iter.collect_vec();
        let history_items = history
            .iter()
            .rev()
            .take(series.num_candles() - 1)
            .rev()
            .chain(series.working_candle.iter())
            .collect_vec();

        assert_eq!(iter_items, history_items);
        assert_eq!(iter_items.len(), series.num_candles());
        assert_eq!(series.capacity(), buffer_capacity);
        assert_eq!(series.num_candles(), buffer_capacity + 1); // +1 for the working candle
    }

    #[test]
    fn test_iter_front_to_back() {
        use super::*;
        use crate::{candle::Candle, timeframe::Timeframe};

        let timeframe = Timeframe::H1;
        let num_candles = 10;
        let buffer_capacity = 5;

        let mut series = CandleSeries::new(Ustr::from("BTCUSDT"), Timeframe::H1, None, buffer_capacity, 89, true, false, false);

        // The initial candle that the series will be "pushing away" from
        let initial_open_time = remove_seconds(Utc::now());
        let initial_candle = Candle {
            symbol:                       Ustr::from("BTCUSDT"),
            timeframe:                    Timeframe::H1,
            open_time:                    initial_open_time,
            close_time:                   initial_open_time + timeframe.duration() - timeframe.close_time_offset(),
            open:                         100.0,
            high:                         100.0,
            low:                          100.0,
            close:                        100.0,
            volume:                       100.0,
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     true,
        };

        // Pushing the initial candle
        series.push_candle(initial_candle).unwrap();

        // The last candle that was pushed into the series
        let mut last_candle = initial_candle;
        let mut history = vec![last_candle.clone()];

        // Populate with dummy candles
        for i in 0..num_candles {
            let new_open_time = last_candle.open_time + timeframe.duration();
            let new_close_time = new_open_time + last_candle.timeframe.duration() - last_candle.timeframe.close_time_offset();

            let new_candle = Candle {
                symbol:                       last_candle.symbol,
                timeframe:                    last_candle.timeframe,
                open_time:                    new_open_time,
                close_time:                   new_close_time,
                open:                         last_candle.open,
                high:                         last_candle.high,
                low:                          last_candle.low,
                close:                        last_candle.close + 1.0,
                volume:                       100.0,
                number_of_trades:             0,
                quote_asset_volume:           0.0,
                taker_buy_quote_asset_volume: 0.0,
                taker_buy_base_asset_volume:  0.0,
                is_final:                     !(i == num_candles - 1),
            };

            series.push_candle(new_candle).unwrap();

            if new_candle.is_final {
                history.push(new_candle.clone());
            }

            last_candle = new_candle;
        }

        let mut iter = series.iter_candles_front_to_back();
        let iter_items = iter.collect_vec();
        let history_items = history
            .iter()
            .rev()
            .take(series.num_candles())
            .chain(series.working_candle.iter())
            .collect_vec();

        for (i, (a, b)) in iter_items.iter().zip(history_items.iter()).enumerate() {
            eprintln!("{}\n{:?}\n{:?}", i, a, b);
            println!();
        }

        assert_eq!(iter_items, history_items);
        assert_eq!(iter_items.len(), series.num_candles());
        assert_eq!(series.capacity(), buffer_capacity);
        assert_eq!(series.num_candles(), buffer_capacity + 1); // +1 for the working candle
    }

    #[test]
    fn test_base_timeframe_aggregation() {
        let symbol = Ustr::from("BTCUSDT");
        let base_timeframe = Timeframe::M1;
        let main_timeframe = Timeframe::M5;
        let buffer_capacity = 10;
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, Some(base_timeframe), buffer_capacity, 89, false, false, false);

        // Simulate pushing 5 base timeframe candles to aggregate into one main timeframe candle.
        let start_time = truncate_to_midnight(Utc::now());

        let mut total_volume: ValueType = 0.0;
        let mut total_number_of_trades = 0;

        // ----------------------------------------------------------------------------
        // Pushing 5 base timeframe candles to aggregate into one main timeframe candle.
        // ----------------------------------------------------------------------------

        for i in 0..5 {
            let open_time = start_time + base_timeframe.duration() * i;

            let candle = Candle {
                symbol: symbol.clone(),
                timeframe: base_timeframe,
                open_time,
                close_time: open_time + base_timeframe.duration() - base_timeframe.close_time_offset(),
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 100.0 + i as f64,
                volume: 1.0 + i as f64,
                number_of_trades: 10 + i as i64,
                is_final: true,
                ..Default::default()
            };

            total_volume += candle.volume;
            total_number_of_trades += candle.number_of_trades;

            eprintln!("VOL: 1.0 + i as f64 = {:#?}", 1.0 + i as f64);
            eprintln!("TRADES: 10 + i as i64 = {:#?}", 10 + i as i64);
            eprintln!("total_volume = {:#?}", total_volume);
            eprintln!("total_number_of_trades = {:#?}", total_number_of_trades);

            series.push_candle(candle).unwrap();
        }

        let aggregated_candle = series.head_candle().unwrap();

        eprintln!("series = {:#?}", series);
        eprintln!("total_volume = {:#?}", total_volume);
        eprintln!("total_number_of_trades = {:#?}", total_number_of_trades);

        // Verify that the series has aggregated the base timeframe candles correctly.
        assert_eq!(series.candles.len(), 1, "There should be one aggregated candle in the series.");

        assert_eq!(aggregated_candle.open, 100.0, "The open price of the aggregated candle should match the first base candle.");
        assert_eq!(aggregated_candle.close, 104.0, "The close price of the aggregated candle should match the last base candle.");
        assert_eq!(aggregated_candle.high, 105.0 + 4.0, "The high price of the aggregated candle should be the highest of the base candles.");
        assert_eq!(aggregated_candle.low, 95.0, "The low price of the aggregated candle should be the lowest of the base candles.");
        assert_eq!(aggregated_candle.volume, 15.0, "The volume should be the sum of the base candles' volumes.");
        assert_eq!(aggregated_candle.number_of_trades, 60, "The number of trades should be the sum of the base candles' number of trades.");
    }

    #[test]
    fn test_main_timeframe_direct_addition() {
        let symbol = Ustr::from("BTCUSDT");
        let main_timeframe = Timeframe::H1;
        let capacity = 10;
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, None, capacity, 89, true, false, false);

        // ----------------------------------------------------------------------------
        // Pushing a non-final working candle and then a finalized main timeframe candle directly.
        // ----------------------------------------------------------------------------
        let initial_open_time = Utc::now();

        let non_final_candle = Candle {
            symbol: symbol.clone(),
            timeframe: main_timeframe,
            open_time: initial_open_time,
            close_time: initial_open_time + main_timeframe.duration() - main_timeframe.close_time_offset(),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 2000.0,
            number_of_trades: 200,
            is_final: false,
            ..Default::default()
        };

        series.push_candle(non_final_candle.clone()).unwrap();

        assert_eq!(series.num_candles(), 1, "Working candle counts as a candle in the series.");
        assert_eq!(series.working_candle.as_ref().unwrap(), &non_final_candle, "The working candle should match the pushed candle.");

        // Finalize the working candle
        let final_candle = Candle {
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 110.0,
            volume: 3000.0,
            number_of_trades: 200,
            is_final: true,
            ..non_final_candle.clone()
        };

        // Pushing a finalized main timeframe candle directly.
        series.push_candle(final_candle.clone()).unwrap();

        // Verify that the candle has been added directly to the series.
        assert_eq!(
            series.candles.len(),
            1,
            "Should be just 1 because the now previous working candle is now finalized and added to the series but no new working candle."
        );

        let direct_added_candle = series.candles.head().unwrap();
        assert_ne!(direct_added_candle, &non_final_candle, "The directly added candle should not match the pushed candle.");
        assert_eq!(direct_added_candle, &final_candle, "The directly added candle should match the pushed candle.");

        // ----------------------------------------------------------------------------
        // Pushing another non-final working candle and then a finalized main timeframe candle directly.
        // ----------------------------------------------------------------------------

        let open_time = initial_open_time + main_timeframe.duration();

        let non_final_candle = Candle {
            symbol: symbol.clone(),
            timeframe: main_timeframe,
            open_time,
            close_time: open_time + main_timeframe.duration() - main_timeframe.close_time_offset(),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 110.0,
            volume: 2000.0,
            number_of_trades: 200,
            is_final: false,
            ..Default::default()
        };

        series.push_candle(non_final_candle.clone()).unwrap();

        assert_eq!(series.num_candles(), 2, "Must be 1 in the series and 1 working candle.");
        assert_eq!(series.working_candle.as_ref().unwrap(), &non_final_candle, "The working candle should match the pushed candle.");

        // Finalize the working candle
        let final_candle = Candle {
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 115.0,
            volume: 3000.0,
            number_of_trades: 200,
            is_final: true,
            ..non_final_candle.clone()
        };

        // Pushing a finalized main timeframe candle directly.
        series.push_candle(final_candle.clone()).unwrap();

        // Verify that the candle has been added directly to the series.
        assert_eq!(
            series.candles.len(),
            2,
            "Should be just 2 because the now previous working candle is now finalized and added to the series but no new working candle."
        );

        let direct_added_candle = series.candles.head().unwrap();
        assert_ne!(direct_added_candle, &non_final_candle, "The directly added candle should not match the pushed candle.");
        assert_eq!(direct_added_candle, &final_candle, "The directly added candle should match the pushed candle.");
    }

    #[test]
    fn test_base_timeframe_with_non_final_updates() {
        let symbol = Ustr::from("BTCUSDT");
        let base_timeframe = Timeframe::M1;
        let main_timeframe = Timeframe::M5;
        let buffer_capacity = 10;
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, Some(base_timeframe), buffer_capacity, 89, true, false, false);

        let start_time = truncate_to_midnight(Utc::now());

        let mut total_volume: ValueType = 0.0;
        let mut total_number_of_trades = 0;

        // Push 3 final base timeframe candles.
        for i in 0..3 {
            let open_time = start_time + base_timeframe.duration() * i;

            let candle = Candle {
                symbol: symbol.clone(),
                timeframe: base_timeframe,
                open_time,
                close_time: open_time + base_timeframe.duration() - base_timeframe.close_time_offset(),
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 100.0 + i as f64,
                volume: 1.0 + i as f64,
                number_of_trades: 10 + i as i64,
                is_final: true,
                ..Default::default()
            };

            total_volume += candle.volume;
            total_number_of_trades += candle.number_of_trades;

            series.push_candle(candle).unwrap();
        }

        // Update the 4th candle with non-final updates.
        let updates = vec![0.5, 0.2, 0.3]; // Simulated incremental volume updates for the 4th candle.
        let open_time = start_time + base_timeframe.duration() * 3;
        for update in updates {
            let candle = Candle {
                symbol: symbol.clone(),
                timeframe: base_timeframe,
                open_time,
                close_time: open_time + base_timeframe.duration() - base_timeframe.close_time_offset(),
                open: 101.0,
                high: 106.0,
                low: 95.0,
                close: 103.0 + update, // Simulate close price variation
                volume: 1.0 + update,  // Increment volume with each update
                number_of_trades: 11,
                is_final: false, // Non-final updates
                ..Default::default()
            };

            series.push_candle(candle).unwrap();
        }

        // Finalize the 4th candle.
        let final_4th_candle = Candle {
            symbol: symbol.clone(),
            timeframe: base_timeframe,
            open_time,
            close_time: open_time + base_timeframe.duration() - base_timeframe.close_time_offset(),
            open: 101.0,
            high: 106.0,
            low: 95.0,
            close: 104.0,
            volume: 3.0,
            number_of_trades: 33,
            is_final: true,
            ..Default::default()
        };
        total_volume += final_4th_candle.volume;
        total_number_of_trades += final_4th_candle.number_of_trades;
        series.push_candle(final_4th_candle).unwrap();

        // Push a 5th final base timeframe candle.
        let open_time = start_time + base_timeframe.duration() * 4;
        let final_5th_candle = Candle {
            symbol: symbol.clone(),
            timeframe: base_timeframe,
            open_time,
            close_time: open_time + base_timeframe.duration() - base_timeframe.close_time_offset(),
            open: 101.0,
            high: 105.0,
            low: 92.0,
            close: 105.0,
            volume: 5.0,
            number_of_trades: 55,
            is_final: true,
            ..Default::default()
        };

        total_volume += final_5th_candle.volume;
        total_number_of_trades += final_5th_candle.number_of_trades;

        // Push the 5th final base timeframe candle.
        series.push_candle(final_5th_candle).unwrap();

        eprintln!("series = {:#?}", series);

        let aggregated_candle = series.candles.head().unwrap();

        // Assertions to verify the aggregated candle matches expected values.
        assert_eq!(series.candles.len(), 1, "There should be one aggregated candle in the series.");
        assert_eq!(aggregated_candle.open, 100.0, "The open price of the aggregated candle should match the first base candle.");
        assert_eq!(aggregated_candle.close, 105.0, "The close price of the aggregated candle should match the last base candle's close.");
        assert_eq!(aggregated_candle.high, 107.0, "The high price of the aggregated candle should be the highest of the base candles.");
        assert_eq!(aggregated_candle.low, 92.0, "The low price of the aggregated candle should be the lowest of the base candles.");
        assert_eq!(aggregated_candle.volume, total_volume, "The volume should be the sum of the base candles' volumes.");
        assert_eq!(aggregated_candle.number_of_trades, total_number_of_trades, "The number of trades should be the sum of the base candles' number of trades.");
    }

    #[test]
    fn test_calculate_candle_start_time() {
        // Create a new series to test the function
        let mut series = CandleSeries::new(Ustr::from("BTCUSDT"), Timeframe::M1, None, 10, 89, true, false, false);

        // Test Case 1
        let trade_time = Utc
            .with_ymd_and_hms(2023, 2, 23, 16, 47, 34)
            .unwrap()
            .with_nanosecond(500_000)
            .expect("Invalid timestamp");

        let timeframe_ms = 100; // 100ms timeframe
        let candle_start_time = calculate_candle_start_time(trade_time, timeframe_ms);

        // Expected start time calculation
        let expected_start_time = Utc.with_ymd_and_hms(2023, 2, 23, 16, 47, 34).unwrap(); // 16:47:34.000Z as the expected start time

        assert_eq!(candle_start_time, expected_start_time, "The candle start time does not match the expected value for Test Case 1.");

        // Test Case 2 - Edge case over midnight
        let trade_time_edge = Utc
            .with_ymd_and_hms(2023, 2, 23, 0, 0, 0)
            .unwrap()
            .with_nanosecond(100_000)
            .expect("Invalid timestamp"); // Just after midnight

        let candle_start_time_edge = calculate_candle_start_time(trade_time_edge, timeframe_ms);

        // Expected start time calculation for the edge case
        let expected_start_time_edge = Utc.with_ymd_and_hms(2023, 2, 23, 0, 0, 0).unwrap(); // 00:00:00Z as the expected start time

        assert_eq!(candle_start_time_edge, expected_start_time_edge, "The candle start time does not match the expected value for Test Case 2.");
    }

    #[test]
    fn test_subsecond_timeframe() {
        let symbol = Ustr::from("BTCUSDT");
        let timeframe = Timeframe::MS100;
        let buffer_capacity = 10;
        let mut series = CandleSeries::new(symbol.clone(), timeframe, None, buffer_capacity, 89, true, false, false);

        // ----------------------------------------------------------------------------
        // Pushing a non-final working candle and then a finalized main timeframe candle directly.
        // ----------------------------------------------------------------------------
        let initial_open_time = truncate_to_midnight(Utc::now());

        let non_final_candle = Candle {
            symbol: symbol.clone(),
            timeframe: timeframe,
            open_time: initial_open_time,
            close_time: initial_open_time + timeframe.duration() - timeframe.close_time_offset(),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 2000.0,
            number_of_trades: 200,
            is_final: false,
            ..Default::default()
        };

        series.push_candle(non_final_candle.clone()).unwrap();

        assert_eq!(series.num_candles(), 1, "Working candle counts as a candle in the series.");
        assert_eq!(series.working_candle.as_ref().unwrap(), &non_final_candle, "The working candle should match the pushed candle.");

        // Finalize the working candle
        let final_candle = Candle {
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 110.0,
            volume: 3000.0,
            number_of_trades: 200,
            is_final: true,
            ..non_final_candle.clone()
        };

        // Pushing a finalized main timeframe candle directly.
        series.push_candle(final_candle.clone()).unwrap();

        // Verify that the candle has been added directly to the series.
        assert_eq!(
            series.candles.len(),
            1,
            "Should be just 1 because the now previous working candle is now finalized and added to the series but no new working candle."
        );

        let direct_added_candle = series.candles.head().unwrap();
        assert_ne!(direct_added_candle, &non_final_candle, "The directly added candle should not match the pushed candle.");
        assert_eq!(direct_added_candle, &final_candle, "The directly added candle should match the pushed candle.");

        // ----------------------------------------------------------------------------
        // Pushing another non-final working candle and then a finalized main timeframe candle directly.
        // ----------------------------------------------------------------------------
        let open_time = initial_open_time + timeframe.duration();

        let non_final_candle = Candle {
            symbol: symbol.clone(),
            timeframe: timeframe,
            open_time,
            close_time: open_time + timeframe.duration() - timeframe.close_time_offset(),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 110.0,
            volume: 2000.0,
            number_of_trades: 200,
            is_final: false,
            ..Default::default()
        };

        series.push_candle(non_final_candle.clone()).unwrap();

        assert_eq!(series.num_candles(), 2, "Must be 1 in the series and 1 working candle.");
        assert_eq!(series.working_candle.as_ref().unwrap(), &non_final_candle, "The working candle should match the pushed candle.");

        // Finalize the working candle
        let final_candle = Candle {
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 115.0,
            volume: 3000.0,
            number_of_trades: 200,
            is_final: true,
            ..non_final_candle.clone()
        };

        // Pushing a finalized main timeframe candle directly.
        series.push_candle(final_candle.clone()).unwrap();

        // Verify that the candle has been added directly to the series.
        assert_eq!(
            series.candles.len(),
            2,
            "Should be just 2 because the now previous working candle is now finalized and added to the series but no new working candle."
        );

        let direct_added_candle = series.candles.head().unwrap();
        assert_ne!(direct_added_candle, &non_final_candle, "The directly added candle should not match the pushed candle.");
        assert_eq!(direct_added_candle, &final_candle, "The directly added candle should match the pushed candle.");

        eprintln!("series = {:#?}", series);
    }

    #[test]
    fn test_subsecond_timeframe_aggregation() {
        let symbol = Ustr::from("BTCUSDT");
        let base_timeframe = Timeframe::MS100;
        let main_timeframe = Timeframe::MS500;
        let buffer_capacity = 10;
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, Some(base_timeframe), buffer_capacity, 89, true, false, false);

        // Simulate pushing 5 base timeframe candles to aggregate into one main timeframe candle.
        let start_time = truncate_to_midnight(Utc::now());

        let mut total_volume: ValueType = 0.0;
        let mut total_number_of_trades = 0;

        // ----------------------------------------------------------------------------
        // Pushing 5 base timeframe candles to aggregate into one main timeframe candle.
        // ----------------------------------------------------------------------------

        for i in 0..5 {
            let open_time = start_time + base_timeframe.duration() * i;

            let candle = Candle {
                symbol: symbol.clone(),
                timeframe: base_timeframe,
                open_time,
                close_time: open_time + base_timeframe.duration() - base_timeframe.close_time_offset(),
                open: 100.0 + i as f64,
                high: 105.0 + i as f64,
                low: 95.0 + i as f64,
                close: 100.0 + i as f64,
                volume: 1.0 + i as f64,
                number_of_trades: 10 + i as i64,
                is_final: true,
                ..Default::default()
            };

            total_volume += candle.volume;
            total_number_of_trades += candle.number_of_trades;

            eprintln!("VOL: 1.0 + i as f64 = {:#?}", 1.0 + i as f64);
            eprintln!("TRADES: 10 + i as i64 = {:#?}", 10 + i as i64);
            eprintln!("total_volume = {:#?}", total_volume);
            eprintln!("total_number_of_trades = {:#?}", total_number_of_trades);

            series.push_candle(candle).unwrap();
        }

        let aggregated_candle = series.candles.head().unwrap();

        eprintln!("series = {:#?}", series);
        eprintln!("total_volume = {:#?}", total_volume);
        eprintln!("total_number_of_trades = {:#?}", total_number_of_trades);

        // Verify that the series has aggregated the base timeframe candles correctly.
        assert_eq!(series.candles.len(), 1, "There should be one aggregated candle in the series.");

        assert_eq!(aggregated_candle.open, 100.0, "The open price of the aggregated candle should match the first base candle.");
        assert_eq!(aggregated_candle.close, 104.0, "The close price of the aggregated candle should match the last base candle.");
        assert_eq!(aggregated_candle.high, 105.0 + 4.0, "The high price of the aggregated candle should be the highest of the base candles.");
        assert_eq!(aggregated_candle.low, 95.0, "The low price of the aggregated candle should be the lowest of the base candles.");
        assert_eq!(aggregated_candle.volume, 15.0, "The volume should be the sum of the base candles' volumes.");
        assert_eq!(aggregated_candle.number_of_trades, 60, "The number of trades should be the sum of the base candles' number of trades.");
    }

    #[test]
    fn test_subsecond_timeframe_aggregation_from_trades_only() {
        let symbol = Ustr::from("BTCUSDT");
        let main_timeframe = Timeframe::MS100;
        let buffer_capacity = 10;
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, None, buffer_capacity, 89, true, false, false);

        let mut last_trade_time = truncate_to_midnight(Utc::now());
        let mut total_volume: ValueType = 0.0;
        let mut total_number_of_trades = 0;

        for i in 0..15 {
            // let trade_time = if i % 3 == 0 { last_trade_time + Duration::milliseconds(100) } else { last_trade_time + Duration::milliseconds(10) };
            let trade_time = last_trade_time + Duration::milliseconds(30);

            eprintln!("trade_time = {:#?}", trade_time);

            let trade = ExchangeTradeEvent {
                event_time: trade_time,
                trade_id: 0,
                symbol: symbol.clone(),
                trade_time,
                price: f!(100.0 + i as f64),
                quantity: f!(1.0 + i as f64),
                is_buyer_maker: true,
            };

            total_volume += trade.quantity.0;
            total_number_of_trades += 1;

            series.apply_trade(trade).unwrap();

            eprintln!("VOL: 1.0 + i as f64 = {:#?}", 1.0 + i as f64);
            eprintln!("TRADES: 1 = {:#?}", 1);
            eprintln!("total_volume = {:#?}", total_volume);
            eprintln!("total_number_of_trades = {:#?}", total_number_of_trades);

            last_trade_time = trade_time;
        }

        eprintln!("series = {:#?}", series);
    }

    #[test]
    fn test_indicators_and_moving_averages_basic() {
        let symbol = Ustr::from("BTCUSDT");
        let main_timeframe = Timeframe::M1;
        let buffer_capacity = FIB_SEQUENCE[5];
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, None, buffer_capacity, 89, true, false, false);

        // ----------------------------------------------------------------------------
        // Populating the series with candles
        // ----------------------------------------------------------------------------
        let initial_open_time = truncate_to_midnight(Utc::now());

        for i in 0..buffer_capacity {
            let open_time = initial_open_time + main_timeframe.duration() * i as i32;

            series
                .push_candle(Candle {
                    symbol: symbol.clone(),
                    timeframe: main_timeframe,
                    open_time,
                    close_time: open_time + main_timeframe.duration() - main_timeframe.close_time_offset(),
                    open: 100.0 + i as f64,
                    high: 105.0 + i as f64,
                    low: 95.0 + i as f64,
                    close: 100.0 + i as f64,
                    volume: 1.0 + i as f64,
                    number_of_trades: 10 + i as i64,
                    is_final: true,
                    ..Default::default()
                })
                .unwrap();
        }

        println!("{:#?}", series);
    }

    #[test]
    fn test_push_candles_from_database() {
        let symbol = Ustr::from("BTCUSDT");
        let main_timeframe = Timeframe::M1;
        let buffer_capacity = FIB_SEQUENCE[15];
        let mut series = CandleSeries::new(symbol.clone(), main_timeframe, None, buffer_capacity, 89, true, false, false);

        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let mut db = Database::new(config.database.clone()).unwrap();
        let candles = db.get_recent_candles(symbol, main_timeframe, buffer_capacity).unwrap();

        eprintln!("candles.len() = {:#?}", candles.len());
        eprintln!("candles.last() = {:#?}", candles.last());
        eprintln!("candles.first() = {:#?}", candles.first());

        for candle in candles {
            series.push_candle(candle).unwrap();
        }

        eprintln!("series.tail_candle() = {:#?}", series.tail_candle());
        eprintln!("series.head_candle() = {:#?}", series.head_candle());
        eprintln!("series.len() = {:#?}", series.num_candles());

        let len = series.num_candles();

        // println!("{:#?}", series.iter_states_back_to_front().skip(len - 10).collect_vec());
        println!("{:#?}", series.working_state);
    }
}
