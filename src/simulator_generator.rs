use crate::{
    appraiser::Appraiser,
    candle::Candle,
    candle_series::CandleSeries,
    f,
    helpers::symbol_for_testing,
    simulator::SimulatorError,
    timeframe::Timeframe,
    timeset::TimeSet,
    types::OrderedValueType,
    util::{closed_candle, remove_milliseconds, remove_seconds, truncate_to_midnight},
};
use chrono::{DateTime, Duration, NaiveDateTime, SubsecRound, Utc};
use eframe::epaint::Stroke;
use egui::{plot::CandleElem, Color32};
use itertools::Itertools;
use log::info;
use num::complex::ComplexFloat;
use ordered_float::*;
use rand::{prelude::ThreadRng, rngs::OsRng, thread_rng, Rng};
use rand_distr::{Distribution, LogNormal, Normal, StudentT};
use std::ops::{Add, RangeInclusive};
use ustr::{ustr, Ustr};
use yata::{
    core::{IndicatorResult, MovingAverage, MovingAverageConstructor, Source, ValueType},
    helpers::{MAInstance, MA},
    indicators::{
        AverageDirectionalIndex, AwesomeOscillator, ChaikinMoneyFlow, CommodityChannelIndex, EaseOfMovement, EldersForceIndex, Kaufman, KnowSureThing,
        MomentumIndex, MoneyFlowIndex, ParabolicSAR, PivotReversalStrategy, PriceChannelStrategy, RelativeStrengthIndex, RelativeVigorIndex,
        SMIErgodicIndicator, StochasticOscillator, TrendStrengthIndex, Trix, TrueStrengthIndex, MACD,
    },
    prelude::*,
};

#[derive(Debug, Clone)]
pub struct GeneratorTimeframeState {
    pub last_observed_base_count: usize,
    pub rotation_interval:        usize,
    pub candles:                  CandleSeries,
    pub ui_candles:               Vec<CandleElem>,
}

#[derive(Debug, Clone, Copy)]
pub struct GeneratorConfigBias {
    pub factor:       ValueType,
    pub max_factor:   ValueType,
    pub target_price: ValueType,
}

#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    pub appraiser:        Appraiser,
    pub base_timeframe:   Timeframe,
    pub timeframes:       Vec<Timeframe>,
    pub initial_price:    ValueType,
    pub stdev:            RangeInclusive<ValueType>,
    pub ticks_per_candle: RangeInclusive<usize>,
    pub series_capacity:  usize,
    pub series_period:    usize,
    pub bias:             Option<GeneratorConfigBias>,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            appraiser:        Appraiser::new(symbol_for_testing().rules.clone()),
            base_timeframe:   Timeframe::M1,
            timeframes:       vec![
                Timeframe::M1,
                Timeframe::M3,
                Timeframe::M5,
                Timeframe::M15,
                Timeframe::M30,
                Timeframe::H1,
                Timeframe::H2,
                Timeframe::H4,
                Timeframe::H6,
                Timeframe::H8,
                Timeframe::H12,
                Timeframe::D1,
                Timeframe::D3,
                Timeframe::W1,
                Timeframe::MM1,
            ],
            initial_price:    100.0,
            stdev:            0.00005..=0.001,
            ticks_per_candle: 5..=100,
            series_capacity:  1000,
            series_period:    121393,
            bias:             None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratorState {
    pub rsi_14:        <RelativeStrengthIndex as IndicatorConfig>::Instance,
    pub rsi_14_wma_14: MAInstance,
    pub cci_18:        <CommodityChannelIndex as IndicatorConfig>::Instance,
    pub cci_18_wma_18: MAInstance,
    pub stoch:         <StochasticOscillator as IndicatorConfig>::Instance,
    pub kama:          <Kaufman as IndicatorConfig>::Instance,
    pub momi:          <MomentumIndex as IndicatorConfig>::Instance,
    pub pcs:           <PriceChannelStrategy as IndicatorConfig>::Instance,
}

#[derive(Debug, Clone, Default)]
pub struct ComputedGeneratorState {
    pub rsi_14:        IndicatorResult,
    pub rsi_14_wma_14: OrderedValueType,
    pub cci_18:        IndicatorResult,
    pub cci_18_wma_18: OrderedValueType,
    pub stoch:         IndicatorResult,
    pub kama:          IndicatorResult,
    pub momi:          IndicatorResult,
    pub pcs:           IndicatorResult,
}

#[derive(Debug, Clone)]
pub struct Generator {
    pub symbol:                   Ustr,
    pub config:                   GeneratorConfig,
    pub tick_counter:             usize,
    pub candle_size:              usize,
    pub current_price:            ValueType,
    pub rotation_intervals:       TimeSet<usize>,
    pub timeframes:               TimeSet<GeneratorTimeframeState>,
    pub computed_generator_state: TimeSet<ComputedGeneratorState>,
    state:                        TimeSet<GeneratorState>,
}

impl Generator {
    pub fn new(config: GeneratorConfig) -> Self {
        let symbol = ustr("TESTUSDT");
        let open_time = remove_milliseconds(truncate_to_midnight(Utc::now()));
        let mut candles = TimeSet::<GeneratorTimeframeState>::new();

        let mut rotation_intervals = TimeSet::<usize>::new();
        rotation_intervals.set(Timeframe::M1, 1);
        rotation_intervals.set(Timeframe::M3, 3);
        rotation_intervals.set(Timeframe::M5, 5);
        rotation_intervals.set(Timeframe::M15, 15);
        rotation_intervals.set(Timeframe::M30, 30);
        rotation_intervals.set(Timeframe::H1, 60);
        rotation_intervals.set(Timeframe::H2, 120);
        rotation_intervals.set(Timeframe::H4, 240);
        rotation_intervals.set(Timeframe::H6, 360);
        rotation_intervals.set(Timeframe::H8, 480);
        rotation_intervals.set(Timeframe::H12, 720);
        rotation_intervals.set(Timeframe::D1, 1440);
        rotation_intervals.set(Timeframe::D3, 4320);
        rotation_intervals.set(Timeframe::W1, 10080);
        rotation_intervals.set(Timeframe::MM1, 43200);

        for timeframe in config.timeframes.iter() {
            let mut series = CandleSeries::new(symbol, *timeframe, None, config.series_capacity, config.series_period, false, false, true);

            let first_candle = Candle {
                symbol,
                timeframe: *timeframe,
                open_time: open_time,
                close_time: open_time + timeframe.duration() - timeframe.close_time_offset(),
                open: config.initial_price,
                high: config.initial_price,
                low: config.initial_price,
                close: config.initial_price,
                volume: 0.0,
                number_of_trades: 0,
                quote_asset_volume: 0.0,
                taker_buy_quote_asset_volume: 0.0,
                taker_buy_base_asset_volume: 0.0,
                is_final: false,
            };

            series.push_candle(first_candle.clone());

            let color = if first_candle.close < first_candle.open { Color32::RED } else { Color32::GREEN };
            let first_ui_candle = CandleElem {
                x:             first_candle.open_time.timestamp_millis() as f64,
                candle:        egui::plot::Candle {
                    timeframe:                    first_candle.timeframe as u64,
                    open_time:                    first_candle.open_time,
                    close_time:                   first_candle.close_time,
                    open:                         first_candle.open,
                    high:                         first_candle.high,
                    low:                          first_candle.low,
                    close:                        first_candle.close,
                    volume:                       first_candle.volume,
                    number_of_trades:             first_candle.number_of_trades as u32,
                    quote_asset_volume:           first_candle.quote_asset_volume,
                    taker_buy_quote_asset_volume: first_candle.taker_buy_quote_asset_volume,
                    taker_buy_base_asset_volume:  first_candle.taker_buy_base_asset_volume,
                },
                candle_width:  *timeframe as u64 as f64 * 0.6,
                whisker_width: *timeframe as u64 as f64 * 0.3,
                stroke:        Stroke::new(1.0, color),
                fill:          color,
            };

            candles.set(
                *timeframe,
                GeneratorTimeframeState {
                    last_observed_base_count: 0,
                    rotation_interval:        *rotation_intervals.get(*timeframe).unwrap(),
                    candles:                  series,
                    ui_candles:               vec![first_ui_candle],
                },
            );
        }

        // ----------------------------------------------------------------------------
        // Initializing the state
        // ----------------------------------------------------------------------------

        let mut state = {
            // Timeset for storing the state of each timeframe
            let mut state = TimeSet::<GeneratorState>::new();

            // Initialize the state for each timeframe with the first candle
            for timeframe in config.timeframes.iter() {
                state.set(
                    *timeframe,
                    GeneratorState {
                        rsi_14: RelativeStrengthIndex {
                            ma:            MA::RMA(14),
                            smooth_ma:     MA::WMA(14),
                            primary_zone:  0.2,
                            smoothed_zone: 0.30,
                            source:        Source::Close,
                        }
                        .init(&closed_candle(config.initial_price))
                        .expect("Failed to initialize RSI"),

                        rsi_14_wma_14: MA::WMA(14).init(0.0).expect("Failed to initialize RSI WMA"),

                        cci_18: CommodityChannelIndex::default()
                            .init(&closed_candle(config.initial_price))
                            .expect("Failed to initialize CCI"),

                        cci_18_wma_18: MA::WMA(18).init(0.0).expect("Failed to initialize CCI WMA"),

                        stoch: StochasticOscillator::default()
                            .init(&closed_candle(config.initial_price))
                            .expect("Failed to initialize Stoch"),

                        kama: Kaufman::default()
                            .init(&closed_candle(config.initial_price))
                            .expect("Failed to initialize KAMA"),

                        momi: MomentumIndex::default()
                            .init(&closed_candle(config.initial_price))
                            .expect("Failed to initialize MOMI"),
                        pcs:  PriceChannelStrategy::default()
                            .init(&closed_candle(config.initial_price))
                            .expect("Failed to initialize PCS"),
                    },
                );
            }

            state
        };

        Self {
            symbol,
            tick_counter: 0,
            candle_size: OsRng.gen_range(config.ticks_per_candle.clone()),
            timeframes: candles,
            current_price: config.initial_price,
            config,
            rotation_intervals,
            state,
            computed_generator_state: TimeSet::new_with(ComputedGeneratorState::default()),
        }
    }

    fn compute_bias_factor(&mut self, current_price: f64, target_price: f64, max_bias: f64) -> f64 {
        let distance = (current_price - target_price).abs();

        // Normalize the distance
        let normalized_distance = distance / target_price;

        // Calculate dynamic bias factor (scaled by max_bias)
        let dynamic_bias = max_bias * normalized_distance;

        // Determine the direction of the bias
        if current_price < target_price {
            dynamic_bias // Positive bias to increase the price
        } else if current_price > target_price {
            -dynamic_bias // Negative bias to decrease the price
        } else {
            0.0
        }
    }

    fn apply_rsi_bias(&mut self, rsi_value: f64, current_price_change: f64) -> f64 {
        let mut rsi_bias_factor = 1.0;

        if rsi_value >= 0.80 {
            // Overbought scenario
            rsi_bias_factor = if OsRng.gen_bool(0.7) { 0.995 } else { 1.005 };
        } else if rsi_value <= 0.20 {
            // Oversold scenario
            rsi_bias_factor = if OsRng.gen_bool(0.7) { 1.005 } else { 0.995 };
        }

        current_price_change * rsi_bias_factor
    }

    fn apply_combined_rsi_bias(&mut self, rsi_values: &[(f64, f64)], current_price_change: f64) -> f64 {
        let mut combined_bias = 1.0;
        let mut total_weight = 0.0;

        for (rsi_value, weight) in rsi_values {
            let rsi_bias_factor = if *rsi_value >= 0.70 {
                if OsRng.gen_bool(0.7) {
                    0.995
                } else {
                    1.005
                }
            } else if *rsi_value <= 0.30 {
                if OsRng.gen_bool(0.7) {
                    1.005
                } else {
                    0.995
                }
            } else {
                1.0
            };

            combined_bias += rsi_bias_factor * weight;
            total_weight += weight;
        }

        if total_weight > 0.0 {
            combined_bias /= total_weight;
        }

        current_price_change * combined_bias
    }

    fn compute_tick(&mut self) -> (ValueType, ValueType) {
        let change_percent = {
            let mean = if let Some(bias) = self.config.bias { self.compute_bias_factor(self.current_price, bias.target_price, bias.max_factor) } else { 0.0 };
            let change_percent = Normal::new(mean, OsRng.gen_range(self.config.stdev.clone())).unwrap().sample(&mut OsRng);

            if OsRng.gen_ratio(1, 5000) {
                change_percent * OsRng.gen_range(10.0..=50.0)
            } else if OsRng.gen_ratio(1, 1000) {
                change_percent * OsRng.gen_range(1.5..=3.0)
            } else {
                change_percent
            }
        };

        // NOTE: Volume is always positive and is affected by the change_percent of the price to simulate real-world volume
        let (volume_lower, volume_upper) = (0.1 * (1.0 + change_percent), 10.0 * (1.0 + change_percent));
        let volume = self.config.appraiser.round_base(OsRng.gen_range(volume_lower..=volume_upper));

        // WARNING: Price can't be negative or zero
        let mut new_price = self.config.appraiser.round_quote(self.current_price * (1.0 + change_percent));

        /*
        // Applying biases
        let mut rsi_values = Vec::new();

        for (weight, (timeframe, _)) in self.timeframes.iter().enumerate() {
            let weight = 1.max(weight) as f64;

            if let Ok(computed_state) = self.computed_generator_state.get(timeframe) {
                if !computed_state.rsi_14.is_empty() {
                    rsi_values.push((computed_state.rsi_14.value(0), weight));
                }
            }
        }

        if !rsi_values.is_empty() {
            let new_biased_price = self.apply_combined_rsi_bias(rsi_values.as_slice(), new_price);
            eprintln!("rsi_values = {:#?}", rsi_values);
            eprintln!("new_biased_price = {:#?}", new_biased_price);
            if new_biased_price.is_finite() && !new_biased_price.is_nan() {
                new_price = new_biased_price;
            }
        }
         */

        if let Ok(base_state) = self.computed_generator_state.get(self.config.base_timeframe) {
            if !base_state.rsi_14.is_empty() {
                new_price = self.apply_rsi_bias(base_state.rsi_14.value(0), new_price);
            }
        }

        self.current_price = if new_price > 0.0 {
            new_price
        } else {
            return self.compute_tick();
        };

        (self.current_price, volume)
    }

    fn compute_indicators(&mut self, timeframe: Timeframe, candle: Candle) -> Result<(), SimulatorError> {
        let state = self.state.get_mut(timeframe)?;

        if candle.is_final {
            let rsi_14_result = state.rsi_14.next(&candle);
            let rsi_14_wma_14_value = state.rsi_14_wma_14.next(&rsi_14_result.value(0));

            let cci_18_result = state.cci_18.next(&candle);
            let cci_18_wma_18_value = state.cci_18_wma_18.next(&cci_18_result.value(0));

            let stoch_result = state.stoch.next(&candle);
            let kama_result = state.kama.next(&candle);
            let momi_result = state.momi.next(&candle);
            let pcs_result = state.pcs.next(&candle);

            self.computed_generator_state.get_mut(timeframe)?.rsi_14 = rsi_14_result;
            self.computed_generator_state.get_mut(timeframe)?.rsi_14_wma_14 = f!(rsi_14_wma_14_value);
            self.computed_generator_state.get_mut(timeframe)?.cci_18 = cci_18_result;
            self.computed_generator_state.get_mut(timeframe)?.cci_18_wma_18 = f!(cci_18_wma_18_value);
            self.computed_generator_state.get_mut(timeframe)?.stoch = stoch_result;
            self.computed_generator_state.get_mut(timeframe)?.kama = kama_result;
            self.computed_generator_state.get_mut(timeframe)?.momi = momi_result;
            self.computed_generator_state.get_mut(timeframe)?.pcs = pcs_result;
        } else {
            let rsi_14_result = state.rsi_14.peek_next(&candle);
            let rsi_14_wma_14_value = state.rsi_14_wma_14.peek_next(&rsi_14_result.value(0));

            let cci_18_result = state.cci_18.peek_next(&candle);
            let cci_18_wma_18_value = state.cci_18_wma_18.peek_next(&cci_18_result.value(0));

            let stoch_result = state.stoch.peek_next(&candle);
            let kama_result = state.kama.peek_next(&candle);
            let momi_result = state.momi.peek_next(&candle);
            let pcs_result = state.pcs.peek_next(&candle);

            self.computed_generator_state.get_mut(timeframe)?.rsi_14 = rsi_14_result;
            self.computed_generator_state.get_mut(timeframe)?.rsi_14_wma_14 = f!(rsi_14_wma_14_value);
            self.computed_generator_state.get_mut(timeframe)?.cci_18 = cci_18_result;
            self.computed_generator_state.get_mut(timeframe)?.cci_18_wma_18 = f!(cci_18_wma_18_value);
            self.computed_generator_state.get_mut(timeframe)?.stoch = stoch_result;
            self.computed_generator_state.get_mut(timeframe)?.kama = kama_result;
            self.computed_generator_state.get_mut(timeframe)?.momi = momi_result;
            self.computed_generator_state.get_mut(timeframe)?.pcs = pcs_result;
        }

        Ok(())
    }

    fn compute_base_timeframe(&mut self, price: ValueType, volume: ValueType) -> Result<(), SimulatorError> {
        if self.tick_counter == self.candle_size {
            // Find the current candle to work on
            let current_candle = self.timeframes.get_mut(self.config.base_timeframe)?.candles.head_candle_mut().unwrap();

            // Finalize current candle
            current_candle.is_final = true;

            // ----------------------------------------------------------------------------
            // Create a new candle and push onto the vector
            // ----------------------------------------------------------------------------
            let new_open_time = current_candle.close_time + self.config.base_timeframe.close_time_offset();
            let new_close_time = new_open_time + self.config.base_timeframe.duration() - self.config.base_timeframe.close_time_offset();

            let new_candle = Candle {
                symbol:                       self.symbol,
                timeframe:                    self.config.base_timeframe,
                open_time:                    new_open_time,
                close_time:                   new_close_time,
                open:                         price,
                high:                         price,
                low:                          price,
                close:                        price,
                volume:                       volume,
                number_of_trades:             0,
                quote_asset_volume:           0.0,
                taker_buy_quote_asset_volume: 0.0,
                taker_buy_base_asset_volume:  0.0,
                is_final:                     false,
            };

            let current_candle_cloned = current_candle.clone();
            self.compute_indicators(self.config.base_timeframe, current_candle_cloned)?;

            self.timeframes.get_mut(self.config.base_timeframe)?.candles.push_candle(new_candle);

            // ----------------------------------------------------------------------------
            // Create a new UI candle and push onto the vector
            // ----------------------------------------------------------------------------

            {
                let color = if new_candle.close < new_candle.open { Color32::RED } else { Color32::GREEN };
                let new_ui_candle = CandleElem {
                    x:             new_candle.open_time.timestamp_millis() as f64,
                    candle:        egui::plot::Candle {
                        timeframe:                    new_candle.timeframe as u64,
                        open_time:                    new_candle.open_time,
                        close_time:                   new_candle.close_time,
                        open:                         new_candle.open,
                        high:                         new_candle.high,
                        low:                          new_candle.low,
                        close:                        new_candle.close,
                        volume:                       new_candle.volume,
                        number_of_trades:             new_candle.number_of_trades as u32,
                        quote_asset_volume:           new_candle.quote_asset_volume,
                        taker_buy_quote_asset_volume: new_candle.taker_buy_quote_asset_volume,
                        taker_buy_base_asset_volume:  new_candle.taker_buy_base_asset_volume,
                    },
                    candle_width:  new_candle.timeframe as u64 as f64 * 0.6,
                    whisker_width: new_candle.timeframe as u64 as f64 * 0.3,
                    stroke:        Stroke::new(1.0, color),
                    fill:          color,
                };
                self.timeframes.get_mut(self.config.base_timeframe)?.ui_candles.push(new_ui_candle);
            }

            // Reset tick_counter and generate new candle_size
            self.candle_size = OsRng.gen_range(self.config.ticks_per_candle.clone());
            self.tick_counter = 0;
        } else {
            let current_candle = {
                // Find the current state to work on
                let current_state = self.timeframes.get_mut(self.config.base_timeframe)?;
                let current_candle = current_state.candles.head_candle_mut().unwrap();

                // ----------------------------------------------------------------------------
                // Update current candle with new trade
                // ----------------------------------------------------------------------------

                current_candle.high = current_candle.high.max(price);
                current_candle.low = current_candle.low.min(price);
                current_candle.close = price;
                current_candle.volume = self.config.appraiser.round_base(current_candle.volume + volume);
                self.tick_counter += 1;

                // ----------------------------------------------------------------------------
                // Update current UI candle with new trade
                // ----------------------------------------------------------------------------
                let color = if current_candle.close < current_candle.open { Color32::RED } else { Color32::GREEN };
                let ui_candle = current_state.ui_candles.last_mut().unwrap();
                ui_candle.candle.high = current_candle.high;
                ui_candle.candle.low = current_candle.low;
                ui_candle.candle.close = current_candle.close;
                ui_candle.candle.volume = current_candle.volume;
                ui_candle.stroke.color = color;
                ui_candle.fill = color;

                current_candle.clone()
            };

            self.compute_indicators(self.config.base_timeframe, current_candle)?;
        }

        Ok(())
    }

    fn compute_other_timeframes(&mut self) -> Result<(), SimulatorError> {
        let base_candle = self.timeframes.get(self.config.base_timeframe)?.candles.head_candle().unwrap().clone();
        let num_base_candles = self.timeframes.get(self.config.base_timeframe)?.candles.num_candles();
        let mut candle_backlog = Vec::with_capacity(self.config.timeframes.len() - 1);

        for (timeframe, state) in self.timeframes.iter_mut() {
            // Skip the base timeframe
            if timeframe == self.config.base_timeframe {
                continue;
            }

            {
                let price = state.candles.head_candle().unwrap().close;
                let current_candle = state.candles.head_candle_mut().unwrap();
                let rotation_interval = *self.rotation_intervals.get(timeframe).unwrap();
                let rotation_condition = num_base_candles % rotation_interval == 0 && state.last_observed_base_count < num_base_candles;

                if rotation_condition {
                    // Finalize current candle and create a new one
                    current_candle.is_final = true;

                    // ----------------------------------------------------------------------------
                    // Create a new candle and push onto the vector
                    // ----------------------------------------------------------------------------
                    let new_candle = Candle {
                        symbol:                       self.symbol,
                        timeframe:                    timeframe,
                        open_time:                    current_candle.close_time,
                        close_time:                   current_candle.close_time + timeframe.duration() - timeframe.close_time_offset(),
                        open:                         base_candle.close,
                        high:                         base_candle.close,
                        low:                          base_candle.close,
                        close:                        base_candle.close,
                        volume:                       0.0,
                        number_of_trades:             0,
                        quote_asset_volume:           0.0,
                        taker_buy_quote_asset_volume: 0.0,
                        taker_buy_base_asset_volume:  0.0,
                        is_final:                     false,
                    };

                    // Preserve the current candle to compute indicators
                    candle_backlog.push((timeframe, current_candle.clone()));

                    state.candles.push_candle(new_candle);
                    state.last_observed_base_count = num_base_candles;

                    // ----------------------------------------------------------------------------
                    // Create a new UI candle and push onto the vector
                    // ----------------------------------------------------------------------------
                    let color = if new_candle.close < new_candle.open { Color32::RED } else { Color32::GREEN };

                    state.ui_candles.push(CandleElem {
                        x:             new_candle.open_time.timestamp_millis() as f64,
                        candle:        egui::plot::Candle {
                            timeframe:                    new_candle.timeframe as u64,
                            open_time:                    new_candle.open_time,
                            close_time:                   new_candle.close_time,
                            open:                         new_candle.open,
                            high:                         new_candle.high,
                            low:                          new_candle.low,
                            close:                        new_candle.close,
                            volume:                       new_candle.volume,
                            number_of_trades:             new_candle.number_of_trades as u32,
                            quote_asset_volume:           new_candle.quote_asset_volume,
                            taker_buy_quote_asset_volume: new_candle.taker_buy_quote_asset_volume,
                            taker_buy_base_asset_volume:  new_candle.taker_buy_base_asset_volume,
                        },
                        candle_width:  new_candle.timeframe as u64 as f64 * 0.6,
                        whisker_width: new_candle.timeframe as u64 as f64 * 0.3,
                        stroke:        Stroke::new(1.0, color),
                        fill:          color,
                    });
                } else {
                    // Update current candle with new tick
                    current_candle.high = current_candle.high.max(base_candle.close);
                    current_candle.low = current_candle.low.min(base_candle.close);
                    current_candle.close = base_candle.close;
                    current_candle.volume = self.config.appraiser.round_base(current_candle.volume + base_candle.volume);

                    // Update current UI candle with new tick
                    let ui_candle = state.ui_candles.last_mut().unwrap();
                    let color = if ui_candle.candle.close < ui_candle.candle.open { Color32::RED } else { Color32::GREEN };
                    ui_candle.candle.high = ui_candle.candle.high.max(base_candle.close);
                    ui_candle.candle.low = ui_candle.candle.low.min(base_candle.close);
                    ui_candle.candle.close = base_candle.close;
                    ui_candle.candle.volume = self.config.appraiser.round_base(ui_candle.candle.volume + base_candle.volume);
                    ui_candle.stroke.color = color;
                    ui_candle.fill = color;

                    // Preserve the current candle to compute indicators
                    candle_backlog.push((timeframe, current_candle.clone()));
                }
            }
        }

        // Compute indicators for the preserved candles
        for (timeframe, candle) in candle_backlog {
            self.compute_indicators(timeframe, candle)?;
        }

        Ok(())
    }

    pub fn compute(&mut self) -> Result<(ValueType, ValueType, Vec<Candle>), SimulatorError> {
        let (price, volume) = self.compute_tick();

        self.compute_base_timeframe(price, volume)?;
        self.compute_other_timeframes()?;

        // These are the candles that are generated in the current tick for all timeframes
        // NOTE: These are be working and/or finalized candles
        let candles = self
            .timeframes
            .iter()
            .filter_map(|(timeframe, state)| if let Some(candle) = state.candles.head_candle() { Some(candle.clone()) } else { None })
            .collect_vec();

        Ok((price, volume, candles))
    }

    pub fn generate_candles(&mut self, timeframe: Timeframe, num_candles: usize) -> Result<(), SimulatorError> {
        if !self.config.timeframes.contains(&timeframe) {
            return Err(SimulatorError::UnsupportedTimeframe(timeframe));
        }

        let target_len = self.timeframes.get(timeframe)?.candles.num_candles() + num_candles;

        while self.timeframes.get(timeframe)?.candles.num_candles() < target_len {
            self.compute()?;
        }

        Ok(())
    }

    /// Returns the head candle
    pub fn head(&self, timeframe: Timeframe) -> Result<Candle, SimulatorError> {
        self.timeframes
            .get(timeframe)?
            .candles
            .head_candle()
            .cloned()
            .ok_or(SimulatorError::CandleNotFound(timeframe, 0))
    }

    pub fn head_mut(&mut self, timeframe: Timeframe) -> Result<&mut Candle, SimulatorError> {
        self.timeframes
            .get_mut(timeframe)?
            .candles
            .head_candle_mut()
            .ok_or(SimulatorError::CandleNotFound(timeframe, 0))
    }

    /// Returns a candle at the given index
    pub fn get(&self, timeframe: Timeframe, index: usize) -> Result<Candle, SimulatorError> {
        self.timeframes
            .get(timeframe)?
            .candles
            .get_candle(index)
            .ok_or(SimulatorError::CandleNotFound(timeframe, index))
            .cloned()
    }

    /// Returns a mutable reference to the candle at the given index
    pub fn get_mut(&mut self, timeframe: Timeframe, index: usize) -> Result<&mut Candle, SimulatorError> {
        self.timeframes
            .get_mut(timeframe)?
            .candles
            .get_candle_mut(index)
            .ok_or(SimulatorError::CandleNotFound(timeframe, index))
    }
}

pub fn generate_price_sequence(starting_price: f64, ticks: usize, stdev: f64) -> Vec<f64> {
    let mut rng = thread_rng();
    let dist = Normal::new(0.0, stdev).unwrap();
    let mut prices = Vec::with_capacity(ticks);
    let mut current_price = starting_price;

    for _ in 0..ticks {
        let change_percent = {
            let change_percent = dist.sample(&mut rng);

            if rng.gen_ratio(1, 5000) {
                change_percent * rng.gen_range(10.0..=50.0)
            } else if rng.gen_ratio(1, 1000) {
                change_percent * rng.gen_range(1.5..=3.0)
            } else {
                change_percent
            }
        };

        current_price *= 1.0 + change_percent;
        prices.push(current_price);
    }

    prices
}

pub fn generate_candles(config: GeneratorConfig, timeframe: Timeframe, start_time: DateTime<Utc>) -> Vec<Candle> {
    let start_time = remove_seconds(start_time);
    let rng = &mut thread_rng();
    let prices = generate_price_sequence(config.initial_price, 1000, rng.gen_range(config.stdev.clone()));
    let ticks_per_candle = rng.gen_range(config.ticks_per_candle);
    let mut counter = 0;

    let first_candle = Candle {
        symbol:                       ustr::ustr("TESTCOIN"),
        timeframe:                    timeframe,
        open_time:                    start_time,
        close_time:                   start_time + timeframe.duration() - timeframe.close_time_offset(),
        open:                         prices[0],
        high:                         prices[0],
        low:                          prices[0],
        close:                        prices[0],
        volume:                       0.0,
        number_of_trades:             0,
        quote_asset_volume:           0.0,
        taker_buy_quote_asset_volume: 0.0,
        taker_buy_base_asset_volume:  0.0,
        is_final:                     true,
    };

    let candles = prices
        .chunks(ticks_per_candle)
        .map(|chunks| {
            let open_time = start_time + (first_candle.timeframe.duration() * counter);
            let close_time = open_time + first_candle.timeframe.duration() - first_candle.timeframe.close_time_offset();

            let candles = chunks.iter().skip(1).enumerate().fold(first_candle, |c, (i, &price)| Candle {
                symbol: c.symbol,
                timeframe: c.timeframe,
                open_time,
                close_time,
                open: if i == 0 { price } else { c.open },
                high: c.high.max(price),
                low: c.low.min(price),
                close: price,
                volume: 0.0,
                number_of_trades: 0,
                quote_asset_volume: 0.0,
                taker_buy_quote_asset_volume: 0.0,
                taker_buy_base_asset_volume: 0.0,
                is_final: i == ticks_per_candle - 1,
            });

            counter += 1;
            candles
        })
        .collect_vec();

    // NOTE: Prepend the first candle to the list
    vec![first_candle].into_iter().chain(candles).collect()
}

fn aggregate_candles_by_timeframe(base_candles: &Vec<Candle>, target_timeframe: Timeframe) -> Vec<Candle> {
    let symbol = base_candles.first().unwrap().symbol;

    let aggregation_factor = match target_timeframe {
        Timeframe::M3 => 3,
        Timeframe::M5 => 5,
        Timeframe::M15 => 15,
        Timeframe::M30 => 30,
        Timeframe::H1 => 60,
        Timeframe::H2 => 120,
        Timeframe::H4 => 240,
        Timeframe::H6 => 360,
        Timeframe::H8 => 480,
        Timeframe::H12 => 720,
        Timeframe::D1 => 1440,
        Timeframe::D3 => 4320,
        Timeframe::W1 => 10080,
        Timeframe::MM1 => 43200,
        _ => panic!("Unsupported timeframe"),
    };

    base_candles
        .chunks(aggregation_factor)
        .into_iter()
        .filter_map(|chunk| {
            let open_time = chunk.first()?.open_time;
            let close_time = chunk.last()?.close_time;

            Some(Candle {
                symbol,
                open_time,
                close_time,
                timeframe: target_timeframe,
                open: chunk.first()?.open,
                close: chunk.last()?.close,
                high: chunk.iter().map(|c| f!(c.high)).max()?.0,
                low: chunk.iter().map(|c| f!(c.low)).min()?.0,
                volume: chunk.iter().map(|c| c.volume).sum(),
                number_of_trades: 0,
                quote_asset_volume: 0.0,
                taker_buy_quote_asset_volume: 0.0,
                taker_buy_base_asset_volume: 0.0,
                is_final: true,
            })
        })
        .collect()
}

fn generate_candles_for_all_timeframes(base_candles: Vec<Candle>) -> TimeSet<Vec<Candle>> {
    let mut candles: TimeSet<Vec<Candle>> = TimeSet::new();
    let base_timeframe = base_candles.first().unwrap().timeframe;
    candles.set(base_timeframe, base_candles.clone());

    let timeframes = vec![
        Timeframe::M3,
        Timeframe::M5,
        Timeframe::M15,
        Timeframe::M30,
        Timeframe::H1,
        Timeframe::H2,
        Timeframe::H4,
        Timeframe::H6,
        Timeframe::H8,
        Timeframe::H12,
        Timeframe::D1,
        Timeframe::D3,
        Timeframe::W1,
        Timeframe::MM1,
    ];

    for timeframe in timeframes.iter() {
        candles.set(*timeframe, aggregate_candles_by_timeframe(&base_candles, *timeframe));
    }

    candles
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_distr::{Gamma, LogNormal};
    use std::time::Instant;

    #[test]
    fn test_candle_generation() {
        let mut generator = Generator::new(GeneratorConfig::default());
        let it = Instant::now();
        generator.generate_candles(Timeframe::M1, 10000);
        println!("{:#?}", it.elapsed());
        // let last_3 = generator.lookback(3);
        // let head = generator.head();

        /*
        println!("{:#?}", generator.timeframes);
        eprintln!("generator.head = {:#?}", generator.head(Timeframe::M1).unwrap());

        eprintln!("generator.timeframes.m1.unwrap().candles.len() = {:#?}", generator.timeframes.m1.unwrap().candles.len());
        eprintln!("generator.timeframes.m3.unwrap().candles.len() = {:#?}", generator.timeframes.m3.unwrap().candles.len());
        eprintln!("generator.timeframes.m5.unwrap().candles.len() = {:#?}", generator.timeframes.m5.unwrap().candles.len());
        eprintln!("generator.timeframes.m15.unwrap().candles.len() = {:#?}", generator.timeframes.m15.unwrap().candles.len());
        eprintln!("generator.timeframes.m30.unwrap().candles.len() = {:#?}", generator.timeframes.m30.unwrap().candles.len());
         */
    }

    #[test]
    fn test_distributions() {
        // Student's t-distribution for modeling heavy tails
        let t_dist = StudentT::new(30.0).unwrap();

        println!("Student's t-distribution for modeling heavy tails");
        for _ in 0..100 {
            println!("{:#?}", t_dist.sample(&mut thread_rng()));
        }

        // Log-normal distribution for strictly positive price movements
        let ln_dist = LogNormal::new(0.0, 1.0).unwrap();

        println!("Log-normal distribution for strictly positive price movements");
        for _ in 0..100 {
            println!("{:#?}", ln_dist.sample(&mut thread_rng()));
        }

        // Gamma distribution for modeling stochastic volatility
        let gamma_dist = Gamma::new(2.0, 5.0).unwrap();

        println!("Gamma distribution for modeling stochastic volatility");
        for _ in 0..100 {
            println!("{:#?}", gamma_dist.sample(&mut thread_rng()));
        }
    }
}
