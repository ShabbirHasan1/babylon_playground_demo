use crate::{
    candle::Candle,
    config::{CoinConfig, TimeframeConfig},
    support_resistance::{MaType, SRLevel},
    timeframe::Timeframe,
    timeset::TimeSet,
};
use anyhow::Result;
use itertools::{Itertools, MinMaxResult};
use log::info;
use ordered_float::OrderedFloat;
use std::time::{Duration, Instant};
use yata::{
    core::{Method, MovingAverageConstructor, PeriodType, ValueType, Window},
    helpers::{MAInstance, MA},
};

/// ----------------------------------------------------------------------------
/// NOTE:
/// (ma_type, name, period, instance)
/// Where `ma_type` is either "ema" or "rma", `name` is the name of the
/// MA e.g. "ema_5", `period` is the period of the MA e.g. 5 and `instance`
/// is the actual moving average instance
///
/// IMPORTANT: Explicit fields are used for better locality and direct access to levels
/// ----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct FrozenMovingAverageState {
    pub max_period:  PeriodType,
    pub ema_5:       Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_5:       Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_8:       Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_8:       Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_13:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_13:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_21:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_21:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_34:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_34:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_55:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_55:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_89:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_89:      Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_144:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_144:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_233:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_233:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_377:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_377:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_610:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_610:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_987:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_987:     Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_1597:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_1597:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_2584:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_2584:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_4181:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_4181:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_6765:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_6765:    Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_10946:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_10946:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_17711:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_17711:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_28657:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_28657:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_46368:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_46368:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_75025:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_75025:   Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_121393:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_121393:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_196418:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_196418:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_317811:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_317811:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_514229:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_514229:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_832040:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_832040:  Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub ema_1346269: Option<(&'static str, &'static str, PeriodType, MAInstance)>,
    pub rma_1346269: Option<(&'static str, &'static str, PeriodType, MAInstance)>,
}

impl FrozenMovingAverageState {
    pub fn new(
        timeframe_config: &TimeframeConfig,
        computed_state_window_size: PeriodType,
        trailing_candles: &Vec<Candle>,
    ) -> Result<(Self, Window<ComputedMovingAverageState>, ComputedMovingAverageState)> {
        // IMPORTANT: `computed_state_window_size` is the size of the resulting window after all the calculations.
        // IMPORTANT: `max_period` is taken from number of passed `trailing_candles`.
        let max_period = trailing_candles.len() as PeriodType + 1;

        let first_candle = trailing_candles.first().expect("trailing_candles is empty");
        let second_candle = trailing_candles.iter().nth(1).cloned().expect("failed to get second candle");

        // ----------------------------------------------------------------------------
        // Moving averages (supports and resistances)
        // WARNING: THINK THRICE BEFORE ADDING OR REMOVING ANYTHING
        // ----------------------------------------------------------------------------

        let mut ema_5 = if 5 <= max_period { Some(("ema", "ema_5", 5 as PeriodType, MA::EMA(5).init(first_candle.close).expect("ema_5"))) } else { None };
        let mut rma_5 = if 5 <= max_period { Some(("rma", "rma_5", 5, MA::RMA(5).init(first_candle.close).expect("rma_5"))) } else { None };

        let mut ema_8 = if 8 <= max_period { Some(("ema", "ema_8", 8, MA::EMA(8).init(first_candle.close).expect("ema_8"))) } else { None };
        let mut rma_8 = if 8 <= max_period { Some(("rma", "ema_8", 8, MA::RMA(8).init(first_candle.close).expect("rma_8"))) } else { None };

        let mut ema_13 = if 13 <= max_period { Some(("ema", "ema_13", 13, MA::EMA(13).init(first_candle.close).expect("ema_13"))) } else { None };
        let mut rma_13 = if 13 <= max_period { Some(("rma", "rma_13", 13, MA::RMA(13).init(first_candle.close).expect("rma_13"))) } else { None };

        let mut ema_21 = if 21 <= max_period { Some(("ema", "ema_21", 21, MA::EMA(21).init(first_candle.close).expect("ema_21"))) } else { None };
        let mut rma_21 = if 21 <= max_period { Some(("rma", "rma_21", 21, MA::RMA(21).init(first_candle.close).expect("rma_21"))) } else { None };

        let mut ema_34 = if 34 <= max_period { Some(("ema", "ema_34", 34, MA::EMA(34).init(first_candle.close).expect("ema_34"))) } else { None };
        let mut rma_34 = if 34 <= max_period { Some(("rma", "rma_34", 34, MA::RMA(34).init(first_candle.close).expect("rma_34"))) } else { None };

        let mut ema_55 = if 55 <= max_period { Some(("ema", "ema_55", 55, MA::EMA(55).init(first_candle.close).expect("ema_55"))) } else { None };
        let mut rma_55 = if 55 <= max_period { Some(("rma", "rma_55", 55, MA::RMA(55).init(first_candle.close).expect("rma_55"))) } else { None };

        let mut ema_89 = if 89 <= max_period { Some(("ema", "ema_89", 89, MA::EMA(89).init(first_candle.close).expect("ema_89"))) } else { None };
        let mut rma_89 = if 89 <= max_period { Some(("rma", "rma_89", 89, MA::RMA(89).init(first_candle.close).expect("rma_89"))) } else { None };

        let mut ema_144 = if 144 <= max_period { Some(("ema", "ema_144", 144, MA::EMA(144).init(first_candle.close).expect("ema_144"))) } else { None };
        let mut rma_144 = if 144 <= max_period { Some(("rma", "rma_144", 144, MA::RMA(144).init(first_candle.close).expect("rma_144"))) } else { None };

        let mut ema_233 = if 233 <= max_period { Some(("ema", "ema_233", 233, MA::EMA(233).init(first_candle.close).expect("ema_233"))) } else { None };
        let mut rma_233 = if 233 <= max_period { Some(("rma", "rma_233", 233, MA::RMA(233).init(first_candle.close).expect("rma_233"))) } else { None };

        let mut ema_377 = if 377 <= max_period { Some(("ema", "ema_377", 377, MA::EMA(377).init(first_candle.close).expect("ema_377"))) } else { None };
        let mut rma_377 = if 377 <= max_period { Some(("rma", "rma_377", 377, MA::RMA(377).init(first_candle.close).expect("rma_377"))) } else { None };

        let mut ema_610 = if 610 <= max_period { Some(("ema", "ema_610", 610, MA::EMA(610).init(first_candle.close).expect("ema_610"))) } else { None };
        let mut rma_610 = if 610 <= max_period { Some(("rma", "rma_610", 610, MA::RMA(610).init(first_candle.close).expect("rma_610"))) } else { None };

        let mut ema_987 = if 987 <= max_period { Some(("ema", "ema_987", 987, MA::EMA(987).init(first_candle.close).expect("ema_987"))) } else { None };
        let mut rma_987 = if 987 <= max_period { Some(("rma", "rma_987", 987, MA::RMA(987).init(first_candle.close).expect("rma_987"))) } else { None };

        let mut ema_1597 = if 1597 <= max_period { Some(("ema", "ema_1597", 1597, MA::EMA(1597).init(first_candle.close).expect("ema_1597"))) } else { None };
        let mut rma_1597 = if 1597 <= max_period { Some(("rma", "rma_1597", 1597, MA::RMA(1597).init(first_candle.close).expect("rma_1597"))) } else { None };

        let mut ema_2584 = if 2584 <= max_period { Some(("ema", "ema_2584", 2584, MA::EMA(2584).init(first_candle.close).expect("ema_2584"))) } else { None };
        let mut rma_2584 = if 2584 <= max_period { Some(("rma", "rma_2584", 2584, MA::RMA(2584).init(first_candle.close).expect("rma_2584"))) } else { None };

        let mut ema_4181 = if 4181 <= max_period { Some(("ema", "ema_4181", 4181, MA::EMA(4181).init(first_candle.close).expect("ema_4181"))) } else { None };
        let mut rma_4181 = if 4181 <= max_period { Some(("rma", "rma_4181", 4181, MA::RMA(4181).init(first_candle.close).expect("rma_4181"))) } else { None };

        let mut ema_6765 = if 6765 <= max_period { Some(("ema", "ema_6765", 6765, MA::EMA(6765).init(first_candle.close).expect("ema_6765"))) } else { None };
        let mut rma_6765 = if 6765 <= max_period { Some(("rma", "rma_6765", 6765, MA::RMA(6765).init(first_candle.close).expect("rma_6765"))) } else { None };

        let mut ema_10946 =
            if 10946 <= max_period { Some(("ema", "ema_10946", 10946, MA::EMA(10946).init(first_candle.close).expect("ema_10946"))) } else { None };
        let mut rma_10946 =
            if 10946 <= max_period { Some(("rma", "rma_10946", 10946, MA::RMA(10946).init(first_candle.close).expect("rma_10946"))) } else { None };

        let mut ema_17711 =
            if 17711 <= max_period { Some(("ema", "ema_17711", 17711, MA::EMA(17711).init(first_candle.close).expect("ema_17711"))) } else { None };
        let mut rma_17711 =
            if 17711 <= max_period { Some(("rma", "rma_17711", 17711, MA::RMA(17711).init(first_candle.close).expect("rma_17711"))) } else { None };

        let mut ema_28657 =
            if 28657 <= max_period { Some(("ema", "ema_28657", 28657, MA::EMA(28657).init(first_candle.close).expect("ema_28657"))) } else { None };
        let mut rma_28657 =
            if 28657 <= max_period { Some(("rma", "rma_28657", 28657, MA::RMA(28657).init(first_candle.close).expect("rma_28657"))) } else { None };

        let mut ema_46368 =
            if 46368 <= max_period { Some(("ema", "ema_46368", 46368, MA::EMA(46368).init(first_candle.close).expect("ema_46368"))) } else { None };
        let mut rma_46368 =
            if 46368 <= max_period { Some(("rma", "rma_46368", 46368, MA::RMA(46368).init(first_candle.close).expect("rma_46368"))) } else { None };

        let mut ema_75025 =
            if 75025 <= max_period { Some(("ema", "ema_75025", 75025, MA::EMA(75025).init(first_candle.close).expect("ema_75025"))) } else { None };
        let mut rma_75025 =
            if 75025 <= max_period { Some(("rma", "rma_75025", 75025, MA::RMA(75025).init(first_candle.close).expect("rma_75025"))) } else { None };

        let mut ema_121393 =
            if 121393 <= max_period { Some(("ema", "ema_121393", 121393, MA::EMA(121393).init(first_candle.close).expect("ema_121393"))) } else { None };
        let mut rma_121393 =
            if 121393 <= max_period { Some(("rma", "rma_121393", 121393, MA::RMA(121393).init(first_candle.close).expect("rma_121393"))) } else { None };

        let mut ema_196418 =
            if 196418 <= max_period { Some(("ema", "ema_196418", 196418, MA::EMA(196418).init(first_candle.close).expect("ema_196418"))) } else { None };
        let mut rma_196418 =
            if 196418 <= max_period { Some(("rma", "rma_196418", 196418, MA::RMA(196418).init(first_candle.close).expect("rma_196418"))) } else { None };

        let mut ema_317811 =
            if 317811 <= max_period { Some(("ema", "ema_317811", 317811, MA::EMA(317811).init(first_candle.close).expect("ema_317811"))) } else { None };
        let mut rma_317811 =
            if 317811 <= max_period { Some(("rma", "rma_317811", 317811, MA::RMA(317811).init(first_candle.close).expect("rma_317811"))) } else { None };

        let mut ema_514229 =
            if 514229 <= max_period { Some(("ema", "ema_514229", 514229, MA::EMA(514229).init(first_candle.close).expect("ema_514229"))) } else { None };
        let mut rma_514229 =
            if 514229 <= max_period { Some(("rma", "rma_514229", 514229, MA::RMA(514229).init(first_candle.close).expect("rma_514229"))) } else { None };

        let mut ema_832040 =
            if 832040 <= max_period { Some(("ema", "ema_832040", 832040, MA::EMA(832040).init(first_candle.close).expect("ema_832040"))) } else { None };
        let mut rma_832040 =
            if 832040 <= max_period { Some(("rma", "rma_832040", 832040, MA::RMA(832040).init(first_candle.close).expect("rma_832040"))) } else { None };

        let mut ema_1346269 =
            if 1346269 <= max_period { Some(("ema", "ema_1346269", 1346269, MA::EMA(1346269).init(first_candle.close).expect("ema_1346269"))) } else { None };
        let mut rma_1346269 =
            if 1346269 <= max_period { Some(("rma", "rma_1346269", 1346269, MA::RMA(1346269).init(first_candle.close).expect("rma_1346269"))) } else { None };

        // ----------------------------------------------------------------------------
        // IMPORTANT: Catching up, computing historical values
        // ----------------------------------------------------------------------------

        let it = Instant::now();

        let first_state = ComputedMovingAverageState {
            candle: second_candle.clone(),
            max_period,
            ema_5: ema_5
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_5: rma_5
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_8: ema_8
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_8: rma_8
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_13: ema_13
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_13: rma_13
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_21: ema_21
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_21: rma_21
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_34: ema_34
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_34: rma_34
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_55: ema_55
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_55: rma_55
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_89: ema_89
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_89: rma_89
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_144: ema_144
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_144: rma_144
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_233: ema_233
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_233: rma_233
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_377: ema_377
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_377: rma_377
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_610: ema_610
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_610: rma_610
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_987: ema_987
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_987: rma_987
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_1597: ema_1597
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_1597: rma_1597
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_2584: ema_2584
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_2584: rma_2584
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_4181: ema_4181
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_4181: rma_4181
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_6765: ema_6765
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_6765: rma_6765
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_10946: ema_10946
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_10946: rma_10946
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_17711: ema_17711
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_17711: rma_17711
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_28657: ema_28657
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_28657: rma_28657
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_46368: ema_46368
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_46368: rma_46368
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_75025: ema_75025
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_75025: rma_75025
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_121393: ema_121393
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_121393: rma_121393
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_196418: ema_196418
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_196418: rma_196418
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_317811: ema_317811
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_317811: rma_317811
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_514229: ema_514229
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_514229: rma_514229
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_832040: ema_832040
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_832040: rma_832040
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            ema_1346269: ema_1346269
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            rma_1346269: rma_1346269
                .as_mut()
                .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&second_candle.close)))),

            total_computation_time: it.elapsed(),
        };

        // initializing state window and filling in the first value
        let mut states = Window::new(computed_state_window_size, first_state);

        let mut last_state = first_state;
        let num_candles = trailing_candles.len();

        trailing_candles.iter().enumerate().skip(1).for_each(|(i, c)| {
            let it = Instant::now();

            // ----------------------------------------------------------------------------
            // WARNING: pushing in historical states
            // ----------------------------------------------------------------------------
            let computed_state = ComputedMovingAverageState {
                candle: c.clone(),
                max_period,
                ema_5: ema_5
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_5: rma_5
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_8: ema_8
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_8: rma_8
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_13: ema_13
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_13: rma_13
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_21: ema_21
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_21: rma_21
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_34: ema_34
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_34: rma_34
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_55: ema_55
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_55: rma_55
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_89: ema_89
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_89: rma_89
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_144: ema_144
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_144: rma_144
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_233: ema_233
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_233: rma_233
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_377: ema_377
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_377: rma_377
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_610: ema_610
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_610: rma_610
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_987: ema_987
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_987: rma_987
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_1597: ema_1597
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_1597: rma_1597
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_2584: ema_2584
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_2584: rma_2584
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_4181: ema_4181
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_4181: rma_4181
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_6765: ema_6765
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_6765: rma_6765
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_10946: ema_10946
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_10946: rma_10946
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_17711: ema_17711
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_17711: rma_17711
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_28657: ema_28657
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_28657: rma_28657
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_46368: ema_46368
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_46368: rma_46368
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_75025: ema_75025
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_75025: rma_75025
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_121393: ema_121393
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_121393: rma_121393
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_196418: ema_196418
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_196418: rma_196418
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_317811: ema_317811
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_317811: rma_317811
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_514229: ema_514229
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_514229: rma_514229
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_832040: ema_832040
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_832040: rma_832040
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                ema_1346269: ema_1346269
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::EMA, *period, OrderedFloat(instance.next(&c.close)))),

                rma_1346269: rma_1346269
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| SRLevel::new(MaType::RMA, *period, OrderedFloat(instance.next(&c.close)))),

                total_computation_time: it.elapsed(),
            };

            if i < num_candles - 1 {
                states.push(computed_state);
            } else {
                // NOTE: keeping last computed state to be used as the last (head) state
                last_state = computed_state;
            }
        });

        let frozen_state = Self {
            max_period,
            ema_5,
            rma_5,
            ema_8,
            rma_8,
            ema_13,
            rma_13,
            ema_21,
            rma_21,
            ema_34,
            rma_34,
            ema_55,
            rma_55,
            ema_89,
            rma_89,
            ema_144,
            rma_144,
            ema_233,
            rma_233,
            ema_377,
            rma_377,
            ema_610,
            rma_610,
            ema_987,
            rma_987,
            ema_1597,
            rma_1597,
            ema_2584,
            rma_2584,
            ema_4181,
            rma_4181,
            ema_6765,
            rma_6765,
            ema_10946,
            rma_10946,
            ema_17711,
            rma_17711,
            ema_28657,
            rma_28657,
            ema_46368,
            rma_46368,
            ema_75025,
            rma_75025,
            ema_121393,
            rma_121393,
            ema_196418,
            rma_196418,
            ema_317811,
            rma_317811,
            ema_514229,
            rma_514229,
            ema_832040,
            rma_832040,
            ema_1346269,
            rma_1346269,
        };

        Ok((frozen_state, states, last_state))
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ComputedMovingAverageState {
    pub candle:                 Candle,
    pub max_period:             PeriodType,
    pub ema_5:                  Option<SRLevel>,
    pub rma_5:                  Option<SRLevel>,
    pub ema_8:                  Option<SRLevel>,
    pub rma_8:                  Option<SRLevel>,
    pub ema_13:                 Option<SRLevel>,
    pub rma_13:                 Option<SRLevel>,
    pub ema_21:                 Option<SRLevel>,
    pub rma_21:                 Option<SRLevel>,
    pub ema_34:                 Option<SRLevel>,
    pub rma_34:                 Option<SRLevel>,
    pub ema_55:                 Option<SRLevel>,
    pub rma_55:                 Option<SRLevel>,
    pub ema_89:                 Option<SRLevel>,
    pub rma_89:                 Option<SRLevel>,
    pub ema_144:                Option<SRLevel>,
    pub rma_144:                Option<SRLevel>,
    pub ema_233:                Option<SRLevel>,
    pub rma_233:                Option<SRLevel>,
    pub ema_377:                Option<SRLevel>,
    pub rma_377:                Option<SRLevel>,
    pub ema_610:                Option<SRLevel>,
    pub rma_610:                Option<SRLevel>,
    pub ema_987:                Option<SRLevel>,
    pub rma_987:                Option<SRLevel>,
    pub ema_1597:               Option<SRLevel>,
    pub rma_1597:               Option<SRLevel>,
    pub ema_2584:               Option<SRLevel>,
    pub rma_2584:               Option<SRLevel>,
    pub ema_4181:               Option<SRLevel>,
    pub rma_4181:               Option<SRLevel>,
    pub ema_6765:               Option<SRLevel>,
    pub rma_6765:               Option<SRLevel>,
    pub ema_10946:              Option<SRLevel>,
    pub rma_10946:              Option<SRLevel>,
    pub ema_17711:              Option<SRLevel>,
    pub rma_17711:              Option<SRLevel>,
    pub ema_28657:              Option<SRLevel>,
    pub rma_28657:              Option<SRLevel>,
    pub ema_46368:              Option<SRLevel>,
    pub rma_46368:              Option<SRLevel>,
    pub ema_75025:              Option<SRLevel>,
    pub rma_75025:              Option<SRLevel>,
    pub ema_121393:             Option<SRLevel>,
    pub rma_121393:             Option<SRLevel>,
    pub ema_196418:             Option<SRLevel>,
    pub rma_196418:             Option<SRLevel>,
    pub ema_317811:             Option<SRLevel>,
    pub rma_317811:             Option<SRLevel>,
    pub ema_514229:             Option<SRLevel>,
    pub rma_514229:             Option<SRLevel>,
    pub ema_832040:             Option<SRLevel>,
    pub rma_832040:             Option<SRLevel>,
    pub ema_1346269:            Option<SRLevel>,
    pub rma_1346269:            Option<SRLevel>,
    pub total_computation_time: Duration,
}

impl Default for ComputedMovingAverageState {
    fn default() -> Self {
        Self {
            candle:                 Default::default(),
            max_period:             1346269,
            ema_5:                  Some(SRLevel::new(MaType::EMA, 5, Default::default())),
            rma_5:                  Some(SRLevel::new(MaType::RMA, 5, Default::default())),
            ema_8:                  Some(SRLevel::new(MaType::EMA, 8, Default::default())),
            rma_8:                  Some(SRLevel::new(MaType::RMA, 8, Default::default())),
            ema_13:                 Some(SRLevel::new(MaType::EMA, 13, Default::default())),
            rma_13:                 Some(SRLevel::new(MaType::RMA, 13, Default::default())),
            ema_21:                 Some(SRLevel::new(MaType::EMA, 21, Default::default())),
            rma_21:                 Some(SRLevel::new(MaType::RMA, 21, Default::default())),
            ema_34:                 Some(SRLevel::new(MaType::EMA, 34, Default::default())),
            rma_34:                 Some(SRLevel::new(MaType::RMA, 34, Default::default())),
            ema_55:                 Some(SRLevel::new(MaType::EMA, 55, Default::default())),
            rma_55:                 Some(SRLevel::new(MaType::RMA, 55, Default::default())),
            ema_89:                 Some(SRLevel::new(MaType::EMA, 89, Default::default())),
            rma_89:                 Some(SRLevel::new(MaType::RMA, 89, Default::default())),
            ema_144:                Some(SRLevel::new(MaType::EMA, 144, Default::default())),
            rma_144:                Some(SRLevel::new(MaType::RMA, 144, Default::default())),
            ema_233:                Some(SRLevel::new(MaType::EMA, 233, Default::default())),
            rma_233:                Some(SRLevel::new(MaType::RMA, 233, Default::default())),
            ema_377:                Some(SRLevel::new(MaType::EMA, 377, Default::default())),
            rma_377:                Some(SRLevel::new(MaType::RMA, 377, Default::default())),
            ema_610:                Some(SRLevel::new(MaType::EMA, 610, Default::default())),
            rma_610:                Some(SRLevel::new(MaType::RMA, 610, Default::default())),
            ema_987:                Some(SRLevel::new(MaType::EMA, 987, Default::default())),
            rma_987:                Some(SRLevel::new(MaType::RMA, 987, Default::default())),
            ema_1597:               Some(SRLevel::new(MaType::EMA, 1597, Default::default())),
            rma_1597:               Some(SRLevel::new(MaType::RMA, 1597, Default::default())),
            ema_2584:               Some(SRLevel::new(MaType::EMA, 2584, Default::default())),
            rma_2584:               Some(SRLevel::new(MaType::RMA, 2584, Default::default())),
            ema_4181:               Some(SRLevel::new(MaType::EMA, 4181, Default::default())),
            rma_4181:               Some(SRLevel::new(MaType::RMA, 4181, Default::default())),
            ema_6765:               Some(SRLevel::new(MaType::EMA, 6765, Default::default())),
            rma_6765:               Some(SRLevel::new(MaType::RMA, 6765, Default::default())),
            ema_10946:              Some(SRLevel::new(MaType::EMA, 10946, Default::default())),
            rma_10946:              Some(SRLevel::new(MaType::RMA, 10946, Default::default())),
            ema_17711:              Some(SRLevel::new(MaType::EMA, 17711, Default::default())),
            rma_17711:              Some(SRLevel::new(MaType::RMA, 17711, Default::default())),
            ema_28657:              Some(SRLevel::new(MaType::EMA, 28657, Default::default())),
            rma_28657:              Some(SRLevel::new(MaType::RMA, 28657, Default::default())),
            ema_46368:              Some(SRLevel::new(MaType::EMA, 46368, Default::default())),
            rma_46368:              Some(SRLevel::new(MaType::RMA, 46368, Default::default())),
            ema_75025:              Some(SRLevel::new(MaType::EMA, 75025, Default::default())),
            rma_75025:              Some(SRLevel::new(MaType::RMA, 75025, Default::default())),
            ema_121393:             Some(SRLevel::new(MaType::EMA, 121393, Default::default())),
            rma_121393:             Some(SRLevel::new(MaType::RMA, 121393, Default::default())),
            ema_196418:             Some(SRLevel::new(MaType::EMA, 196418, Default::default())),
            rma_196418:             Some(SRLevel::new(MaType::RMA, 196418, Default::default())),
            ema_317811:             Some(SRLevel::new(MaType::EMA, 317811, Default::default())),
            rma_317811:             Some(SRLevel::new(MaType::RMA, 317811, Default::default())),
            ema_514229:             Some(SRLevel::new(MaType::EMA, 514229, Default::default())),
            rma_514229:             Some(SRLevel::new(MaType::RMA, 514229, Default::default())),
            ema_832040:             Some(SRLevel::new(MaType::EMA, 832040, Default::default())),
            rma_832040:             Some(SRLevel::new(MaType::RMA, 832040, Default::default())),
            ema_1346269:            Some(SRLevel::new(MaType::EMA, 1346269, Default::default())),
            rma_1346269:            Some(SRLevel::new(MaType::RMA, 1346269, Default::default())),
            total_computation_time: Default::default(),
        }
    }
}

impl<'a> ComputedMovingAverageState {
    pub fn iter(&'a self) -> impl Iterator<Item = &'a SRLevel> {
        IntoIterator::into_iter([
            &self.ema_5,
            &self.rma_5,
            &self.ema_8,
            &self.rma_8,
            &self.ema_13,
            &self.rma_13,
            &self.ema_21,
            &self.rma_21,
            &self.ema_34,
            &self.rma_34,
            &self.ema_55,
            &self.rma_55,
            &self.ema_89,
            &self.rma_89,
            &self.ema_144,
            &self.rma_144,
            &self.ema_233,
            &self.rma_233,
            &self.ema_377,
            &self.rma_377,
            &self.ema_610,
            &self.rma_610,
            &self.ema_987,
            &self.rma_987,
            &self.ema_1597,
            &self.rma_1597,
            &self.ema_2584,
            &self.rma_2584,
            &self.ema_4181,
            &self.rma_4181,
            &self.ema_6765,
            &self.rma_6765,
            &self.ema_10946,
            &self.rma_10946,
            &self.ema_17711,
            &self.rma_17711,
            &self.ema_28657,
            &self.rma_28657,
            &self.ema_46368,
            &self.rma_46368,
            &self.ema_75025,
            &self.rma_75025,
            &self.ema_121393,
            &self.rma_121393,
            &self.ema_196418,
            &self.rma_196418,
            &self.ema_317811,
            &self.rma_317811,
            &self.ema_514229,
            &self.rma_514229,
            &self.ema_832040,
            &self.rma_832040,
            &self.ema_1346269,
            &self.rma_1346269,
        ])
        .filter_map(|option| option.as_ref())
    }
}

impl<'a> IntoIterator for &'a ComputedMovingAverageState {
    type Item = Option<SRLevel>;
    type IntoIter = std::array::IntoIter<Self::Item, 54>;

    // WARNING: it copies, not really moving; this is a temporary hack (I hope)
    fn into_iter(self) -> Self::IntoIter {
        IntoIterator::into_iter([
            self.ema_5,
            self.rma_5,
            self.ema_8,
            self.rma_8,
            self.ema_13,
            self.rma_13,
            self.ema_21,
            self.rma_21,
            self.ema_34,
            self.rma_34,
            self.ema_55,
            self.rma_55,
            self.ema_89,
            self.rma_89,
            self.ema_144,
            self.rma_144,
            self.ema_233,
            self.rma_233,
            self.ema_377,
            self.rma_377,
            self.ema_610,
            self.rma_610,
            self.ema_987,
            self.rma_987,
            self.ema_1597,
            self.rma_1597,
            self.ema_2584,
            self.rma_2584,
            self.ema_4181,
            self.rma_4181,
            self.ema_6765,
            self.rma_6765,
            self.ema_10946,
            self.rma_10946,
            self.ema_17711,
            self.rma_17711,
            self.ema_28657,
            self.rma_28657,
            self.ema_46368,
            self.rma_46368,
            self.ema_75025,
            self.rma_75025,
            self.ema_121393,
            self.rma_121393,
            self.ema_196418,
            self.rma_196418,
            self.ema_317811,
            self.rma_317811,
            self.ema_514229,
            self.rma_514229,
            self.ema_832040,
            self.rma_832040,
            self.ema_1346269,
            self.rma_1346269,
        ])
    }
}

impl ComputedMovingAverageState {
    // NOTE: sorting support and resistance levels by its price
    // srs.sort_by(|a, b| a.price.cmp(&b.price));

    // TODO: implement above("ma name")
    // TODO: implement below("ma name")

    pub fn empty(candle: Candle, max_period: PeriodType) -> Self {
        Self {
            candle,
            max_period,
            ema_5: None,
            rma_5: None,
            ema_8: None,
            rma_8: None,
            ema_13: None,
            rma_13: None,
            ema_21: None,
            rma_21: None,
            ema_34: None,
            rma_34: None,
            ema_55: None,
            rma_55: None,
            ema_89: None,
            rma_89: None,
            ema_144: None,
            rma_144: None,
            ema_233: None,
            rma_233: None,
            ema_377: None,
            rma_377: None,
            ema_610: None,
            rma_610: None,
            ema_987: None,
            rma_987: None,
            ema_1597: None,
            rma_1597: None,
            ema_2584: None,
            rma_2584: None,
            ema_4181: None,
            rma_4181: None,
            ema_6765: None,
            rma_6765: None,
            ema_10946: None,
            rma_10946: None,
            ema_17711: None,
            rma_17711: None,
            ema_28657: None,
            rma_28657: None,
            ema_46368: None,
            rma_46368: None,
            ema_75025: None,
            rma_75025: None,
            ema_121393: None,
            rma_121393: None,
            ema_196418: None,
            rma_196418: None,
            ema_317811: None,
            rma_317811: None,
            ema_514229: None,
            rma_514229: None,
            ema_832040: None,
            rma_832040: None,
            ema_1346269: None,
            rma_1346269: None,
            total_computation_time: Default::default(),
        }
    }

    pub fn len(&self) -> usize {
        25
    }

    pub fn min(&self) -> SRLevel {
        match self.into_iter().min() {
            Some(Some(sr)) => sr,
            _ => panic!("min() failed, no moving averages"),
        }
    }

    pub fn max(&self) -> SRLevel {
        match self.into_iter().max() {
            Some(Some(sr)) => sr,
            _ => panic!("max() failed, no moving averages"),
        }
    }

    pub fn minmax(&self) -> (SRLevel, SRLevel) {
        match self.into_iter().minmax() {
            MinMaxResult::NoElements => panic!("minmax failed, no moving averages"),
            MinMaxResult::OneElement(sr) => match sr {
                Some(sr) => (sr, sr),
                _ => panic!("minmax failed, no moving averages"),
            },
            MinMaxResult::MinMax(min, max) => {
                let min = match min {
                    Some(sr) => sr,
                    _ => panic!("minmax failed `min` is none"),
                };
                let max = match max {
                    Some(sr) => sr,
                    _ => panic!("minmax failed `max` is none"),
                };

                (min, max)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        database::{Database, DatabaseBackend},
        f, peek_state,
        types::OrderedValueType,
    };
    use parking_lot::RwLock;
    use rayon::prelude::{IntoParallelRefIterator, *};
    use std::sync::Arc;
    use ustr::ustr;
    use yata::{
        core::{MovingAverage, Source, OHLCV},
        methods::{LinearVolatility, Vidya, ADI, DMA, EMA, HMA, LSMA, MMA, RMA, SMA, SMM, SMMA, SWMA, TRIMA, VWMA, WMA},
    };

    #[test]
    fn test_large_emas() {
        let it = Instant::now();
        let frozen_indicator_state = FrozenMovingAverageState {
            max_period:  4181,
            ema_5:       Some(("ema", "ema_5", 5, MA::EMA(5).init(0.0).expect("ema_5"))),
            rma_5:       Some(("rma", "rma_5", 5, MA::RMA(5).init(0.0).expect("rma_5"))),
            ema_8:       Some(("ema", "ema_8", 8, MA::EMA(8).init(0.0).expect("ema_8"))),
            rma_8:       Some(("rma", "rma_8", 8, MA::RMA(8).init(0.0).expect("rma_8"))),
            ema_13:      Some(("ema", "ema_13", 13, MA::EMA(13).init(0.0).expect("ema_13"))),
            rma_13:      Some(("rma", "rma_13", 13, MA::RMA(13).init(0.0).expect("rma_13"))),
            ema_21:      Some(("ema", "ema_21", 21, MA::EMA(21).init(0.0).expect("ema_21"))),
            rma_21:      Some(("rma", "rma_21", 21, MA::RMA(21).init(0.0).expect("rma_21"))),
            ema_34:      Some(("ema", "ema_34", 34, MA::EMA(34).init(0.0).expect("ema_34"))),
            rma_34:      Some(("rma", "rma_34", 34, MA::RMA(34).init(0.0).expect("rma_34"))),
            ema_55:      Some(("ema", "ema_55", 55, MA::EMA(55).init(0.0).expect("ema_55"))),
            rma_55:      Some(("rma", "rma_55", 55, MA::RMA(55).init(0.0).expect("rma_55"))),
            ema_89:      Some(("ema", "ema_89", 89, MA::EMA(89).init(0.0).expect("ema_89"))),
            rma_89:      Some(("rma", "rma_89", 89, MA::RMA(89).init(0.0).expect("rma_89"))),
            ema_144:     Some(("ema", "ema_144", 144, MA::EMA(144).init(0.0).expect("ema_144"))),
            rma_144:     Some(("rma", "rma_144", 144, MA::RMA(144).init(0.0).expect("rma_144"))),
            ema_233:     Some(("ema", "ema_233", 233, MA::EMA(233).init(0.0).expect("ema_233"))),
            rma_233:     Some(("rma", "rma_233", 233, MA::RMA(233).init(0.0).expect("rma_233"))),
            ema_377:     Some(("ema", "ema_377", 377, MA::EMA(377).init(0.0).expect("ema_377"))),
            rma_377:     Some(("rma", "rma_377", 377, MA::RMA(377).init(0.0).expect("rma_377"))),
            ema_610:     Some(("ema", "ema_610", 610, MA::EMA(610).init(0.0).expect("ema_610"))),
            rma_610:     Some(("rma", "rma_610", 610, MA::RMA(610).init(0.0).expect("rma_610"))),
            ema_987:     Some(("ema", "ema_987", 987, MA::EMA(987).init(0.0).expect("ema_987"))),
            rma_987:     Some(("rma", "rma_987", 987, MA::RMA(987).init(0.0).expect("rma_987"))),
            ema_1597:    Some(("ema", "ema_1597", 1597, MA::EMA(1597).init(0.0).expect("ema_1597"))),
            rma_1597:    Some(("rma", "rma_1597", 1597, MA::RMA(1597).init(0.0).expect("rma_1597"))),
            ema_2584:    Some(("ema", "ema_2584", 2584, MA::EMA(2584).init(0.0).expect("ema_2584"))),
            rma_2584:    Some(("rma", "rma_2584", 2584, MA::RMA(2584).init(0.0).expect("rma_2584"))),
            ema_4181:    Some(("ema", "ema_4181", 4181, MA::EMA(4181).init(0.0).expect("ema_4181"))),
            rma_4181:    Some(("rma", "rma_4181", 4181, MA::RMA(4181).init(0.0).expect("rma_4181"))),
            ema_6765:    Some(("ema", "ema_6765", 6795, MA::EMA(6765).init(0.0).expect("ema_6765"))),
            rma_6765:    Some(("rma", "rma_6765", 6795, MA::RMA(6765).init(0.0).expect("rma_6765"))),
            ema_10946:   Some(("ema", "ema_10946", 10946, MA::EMA(10946).init(0.0).expect("ema_10946"))),
            rma_10946:   Some(("rma", "rma_10946", 10946, MA::RMA(10946).init(0.0).expect("rma_10946"))),
            ema_17711:   Some(("ema", "ema_17711", 17711, MA::EMA(17711).init(0.0).expect("ema_17711"))),
            rma_17711:   Some(("rma", "rma_17711", 17711, MA::RMA(17711).init(0.0).expect("rma_17711"))),
            ema_28657:   Some(("ema", "ema_28657", 28657, MA::EMA(28657).init(0.0).expect("ema_28657"))),
            rma_28657:   Some(("rma", "rma_28657", 28657, MA::RMA(28657).init(0.0).expect("rma_28657"))),
            ema_46368:   Some(("ema", "ema_46368", 46368, MA::EMA(46368).init(0.0).expect("ema_46368"))),
            rma_46368:   Some(("rma", "rma_46368", 46368, MA::RMA(46368).init(0.0).expect("rma_46368"))),
            ema_75025:   Some(("ema", "ema_75025", 75025, MA::EMA(75025).init(0.0).expect("ema_75025"))),
            rma_75025:   Some(("rma", "rma_75025", 75025, MA::RMA(75025).init(0.0).expect("rma_75025"))),
            ema_121393:  Some(("ema", "ema_121393", 121393, MA::EMA(121393).init(0.0).expect("ema_121393"))),
            rma_121393:  Some(("rma", "rma_121393", 121393, MA::RMA(121393).init(0.0).expect("rma_121393"))),
            ema_196418:  Some(("ema", "ema_196418", 196418, MA::EMA(196418).init(0.0).expect("ema_196418"))),
            rma_196418:  Some(("rma", "rma_196418", 196418, MA::RMA(196418).init(0.0).expect("rma_196418"))),
            ema_317811:  Some(("ema", "ema_317811", 317811, MA::EMA(317811).init(0.0).expect("ema_317811"))),
            rma_317811:  Some(("rma", "rma_317811", 317811, MA::RMA(317811).init(0.0).expect("rma_317811"))),
            ema_514229:  Some(("ema", "ema_514229", 514229, MA::EMA(514229).init(0.0).expect("ema_514229"))),
            rma_514229:  Some(("rma", "rma_514229", 514229, MA::RMA(514229).init(0.0).expect("rma_514229"))),
            ema_832040:  Some(("ema", "ema_832040", 832040, MA::EMA(832040).init(0.0).expect("ema_832040"))),
            rma_832040:  Some(("rma", "rma_832040", 832040, MA::RMA(832040).init(0.0).expect("rma_832040"))),
            ema_1346269: Some(("ema", "ema_1346269", 1346269, MA::EMA(1346269).init(0.0).expect("ema_1346269"))),
            rma_1346269: Some(("rma", "rma_1346269", 1346269, MA::RMA(1346269).init(0.0).expect("rma_1346269"))),
        };
        println!("{:#?}", it.elapsed());

        let it = Instant::now();
        let mut moving_averages = Arc::new(RwLock::new(TimeSet {
            s1:  Some(frozen_indicator_state.clone()),
            m1:  Some(frozen_indicator_state.clone()),
            m3:  Some(frozen_indicator_state.clone()),
            m5:  Some(frozen_indicator_state.clone()),
            m15: Some(frozen_indicator_state.clone()),
            m30: Some(frozen_indicator_state.clone()),
            h1:  Some(frozen_indicator_state.clone()),
            h2:  Some(frozen_indicator_state.clone()),
            h4:  Some(frozen_indicator_state.clone()),
            h6:  Some(frozen_indicator_state.clone()),
            h8:  Some(frozen_indicator_state.clone()),
            h12: Some(frozen_indicator_state.clone()),
            d1:  Some(frozen_indicator_state.clone()),
            d3:  Some(frozen_indicator_state.clone()),
            w1:  Some(frozen_indicator_state.clone()),
            mm1: Some(frozen_indicator_state.clone()),
        }));
        println!("{:#?}", it.elapsed());

        let it = Instant::now();
        for i in 1..=1000000 {
            if i % 10 == 0 {
                moving_averages.write().apply(|x| {
                    x.ema_5.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_5.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_8.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_8.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_13.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_13.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_21.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_21.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_34.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_34.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_55.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_55.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_89.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_89.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_144.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_144.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_233.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_233.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_377.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_377.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_610.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_610.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_987.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_987.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_1597.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_1597.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_2584.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_2584.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_4181.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_4181.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_6765.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_6765.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_10946.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_10946.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_17711.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_17711.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_28657.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_28657.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_46368.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_46368.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_75025.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_75025.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_121393.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_121393.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_196418.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_196418.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_317811.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_317811.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_514229.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_514229.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_832040.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_832040.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.ema_1346269.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                    x.rma_1346269.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                });
            } else {
                moving_averages.write().apply(|x| {
                    x.ema_5.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_5.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_8.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_8.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_13.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_13.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_21.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_21.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_34.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_34.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_55.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_55.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_89.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_89.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_144
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_144
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_233
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_233
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_377
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_377
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_610
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_610
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_987
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_987
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_1597
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_1597
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_2584
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_2584
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_4181
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_4181
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_6765
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_6765
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_10946
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_10946
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_17711
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_17711
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_28657
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_28657
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_46368
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_46368
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_75025
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_75025
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_121393
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_121393
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_196418
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_196418
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_317811
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_317811
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_514229
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_514229
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_832040
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_832040
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.ema_1346269
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                    x.rma_1346269
                        .as_mut()
                        .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                });
            }
        }
        println!("catching up {:#?}", it.elapsed());

        let it = Instant::now();
        for i in 1..=500000 {
            moving_averages.write().apply(|x| {
                x.ema_5.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_5.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_8.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_8.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_13.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_13.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_21.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_21.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_34.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_34.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_55.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_55.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_89.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_89.as_mut().map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_144
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_144
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_233
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_233
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_377
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_377
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_610
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_610
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_987
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_987
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_1597
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_1597
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_2584
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_2584
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_4181
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_4181
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_6765
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_6765
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_10946
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_10946
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_17711
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_17711
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_28657
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_28657
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_46368
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_46368
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_75025
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_75025
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_121393
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_121393
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_196418
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_196418
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_317811
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_317811
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_514229
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_514229
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_832040
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_832040
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.ema_1346269
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
                x.rma_1346269
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&(i as ValueType)));
            });
        }
        println!("peeks {:#?}", it.elapsed());

        let it = Instant::now();
        for i in 1..=500000 {
            moving_averages.write().apply(|x| {
                x.ema_5.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_5.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_8.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_8.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_13.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_13.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_21.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_21.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_34.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_34.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_55.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_55.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_89.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_89.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_144.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_144.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_233.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_233.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_377.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_377.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_610.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_610.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_987.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_987.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_1597.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_1597.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_2584.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_2584.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_4181.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_4181.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_6765.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_6765.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_10946.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_10946.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_17711.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_17711.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_28657.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_28657.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_46368.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_46368.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_75025.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_75025.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_121393.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_121393.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_196418.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_196418.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_317811.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_317811.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_514229.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_514229.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_832040.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_832040.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.ema_1346269.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
                x.rma_1346269.as_mut().map(|(ma_type, name, period, instance)| instance.next(&(i as ValueType)));
            });
        }
        println!("mutations {:#?}", it.elapsed());

        moving_averages.write().apply(|x| {
            println!(
                "{}",
                x.ema_1346269
                    .as_mut()
                    .map(|(ma_type, name, period, instance)| instance.peek_next(&68716.0))
                    .unwrap()
            );
        });

        println!("---");

        moving_averages.write().apply(|x| {
            println!("{}", x.ema_1346269.as_mut().map(|(ma_type, name, period, instance)| instance.next(&68716.0)).unwrap());
        });
    }

    #[test]
    fn test_timeframe_to_u64() {
        println!("{:#?}", Timeframe::S1 as u64);
        println!("{:#?}", Timeframe::M1 as u64);
        println!("{:#?}", Timeframe::M3 as u64);
        println!("{:#?}", Timeframe::M5 as u64);
        println!("{:#?}", Timeframe::M15 as u64);
        println!("{:#?}", Timeframe::M30 as u64);
        println!("{:#?}", Timeframe::H1 as u64);
        println!("{:#?}", Timeframe::H2 as u64);
        println!("{:#?}", Timeframe::H4 as u64);
        println!("{:#?}", Timeframe::H6 as u64);
        println!("{:#?}", Timeframe::H8 as u64);
        println!("{:#?}", Timeframe::H12 as u64);
        println!("{:#?}", Timeframe::D1 as u64);
        println!("{:#?}", Timeframe::D3 as u64);
        println!("{:#?}", Timeframe::W1 as u64);
        println!("{:#?}", Timeframe::MM1 as u64);

        assert_eq!(Timeframe::M1 as u64, 60);
    }

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    enum MAType {
        SMA,
        EMA,
        WMA,
        HMA,
        RMA,
    }

    fn optimize_ma(
        data: &Vec<Candle>,
        lower_period: PeriodType,
        upper_period: PeriodType,
        threshold: OrderedValueType,
        top_n: usize,
    ) -> Vec<(MAType, PeriodType, Source, OrderedValueType)> {
        let mut scores: Vec<(MAType, PeriodType, Source, OrderedValueType)> = Vec::new();

        let sources = [Source::High, Source::Low, Source::Close, Source::HL2, Source::TP];

        let all_results: Vec<_> = [MAType::EMA]
            .par_iter()
            .flat_map(|&ma| {
                (lower_period..upper_period).into_par_iter().flat_map(move |period| {
                    sources
                        .par_iter()
                        .map(move |&source| {
                            let score = calculate_score(data, ma, period, source, threshold);
                            (ma, period, source, score)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        scores.extend(all_results);

        // Sort the scores and take the top N
        scores.sort_by(|&(_, _, _, score_a), &(_, _, _, score_b)| score_b.partial_cmp(&score_a).unwrap());
        scores.truncate(top_n);
        scores
    }

    fn calculate_linear_volatility(data: &Vec<Candle>) -> OrderedValueType {
        let diffs: Vec<f64> = data.windows(2).map(|w| w[1].close - w[0].close).collect();
        OrderedFloat(diffs.iter().map(|&diff| diff.abs()).sum::<f64>() / diffs.len() as f64)
    }

    fn calculate_ma(data: &Vec<Candle>, ma_type: MAType, period: PeriodType, source: Source) -> Vec<OrderedValueType> {
        let xs = &data.iter().map(|x| x.source(source)).collect_vec();

        match ma_type {
            MAType::SMA => SMA::new_over(period, &xs).unwrap().into_iter().map(|x| OrderedFloat(x)).collect_vec(),
            MAType::EMA => EMA::new_over(period, &xs).unwrap().into_iter().map(|x| OrderedFloat(x)).collect_vec(),
            MAType::WMA => WMA::new_over(period, &xs).unwrap().into_iter().map(|x| OrderedFloat(x)).collect_vec(),
            MAType::HMA => HMA::new_over(period, &xs).unwrap().into_iter().map(|x| OrderedFloat(x)).collect_vec(),
            MAType::RMA => RMA::new_over(period, &xs).unwrap().into_iter().map(|x| OrderedFloat(x)).collect_vec(),
        }
    }

    fn calculate_score(data: &Vec<Candle>, ma_type: MAType, period: PeriodType, source: Source, threshold: OrderedValueType) -> OrderedValueType {
        let mut touch_count = 0;
        let mut total_score = 0.0;
        let consecutive_count = 3; // adjust based on your needs
        let mut previous_direction: Option<OrderedValueType> = None;
        let mut consistent_direction_count = 0;

        // Computing moving averages
        let ma_data = calculate_ma(data, ma_type, period, source);

        for i in 1..data.len() {
            let high = OrderedFloat(data[i].high);
            let low = OrderedFloat(data[i].low);
            let ma_value = ma_data[i];

            let diff_high = high - ma_value;
            let diff_low = ma_value - low;

            // Checking if the price approaches or touches the MA
            if OrderedFloat(diff_high.abs()) <= threshold || OrderedFloat(diff_low.abs()) <= threshold {
                let direction = if diff_high <= threshold { 1.0 } else { -1.0 };

                if let Some(prev_dir) = previous_direction {
                    if prev_dir == direction {
                        consistent_direction_count += 1;
                    } else {
                        consistent_direction_count = 1;
                    }
                } else {
                    consistent_direction_count = 1;
                }

                if consistent_direction_count < consecutive_count {
                    touch_count += 1;
                    total_score += 1.0 / diff_high.abs().max(diff_low.abs()); // Closer to the MA, higher the score
                }

                previous_direction = Some(OrderedFloat(direction));
            } else {
                previous_direction = None;
                consistent_direction_count = 0;
            }
        }

        // Assuming each touch/test is 10x more valuable than the score from proximity
        OrderedFloat(touch_count as ValueType) + OrderedFloat((total_score * 0.1))
    }

    fn filter_similar_moving_averages(mas: &mut Vec<(MAType, PeriodType, Source, OrderedValueType)>, min_period_diff: PeriodType) {
        // Sort by score descending
        mas.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());

        let mut to_remove = Vec::new();

        for i in 0..mas.len() {
            for j in (i + 1)..mas.len() {
                if mas[i].0 == mas[j].0 && mas[i].2 == mas[j].2 && (mas[i].1 as isize - mas[j].1 as isize).abs() <= min_period_diff as isize {
                    to_remove.push(j);
                }
            }
        }

        to_remove.sort_unstable();
        to_remove.dedup();

        for &index in to_remove.iter().rev() {
            mas.remove(index);
        }
    }

    #[test]
    fn test_moving_averages() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let mut db = Database::new(config.database.clone()).unwrap();
        let candles = db.get_all_candles(ustr("BTCUSDT"), Timeframe::M1).unwrap();

        eprintln!("candles.len() = {:#?}", candles.len());

        let it = Instant::now();
        let linear_volatility = calculate_linear_volatility(&candles);

        let dynamic_threshold = linear_volatility * f!(0.0005);
        let raw_scores = optimize_ma(&candles, 233, 4181, dynamic_threshold, 100);
        let filtered_scores = filter_similar_moving_averages(&mut raw_scores.clone(), 5);

        println!("raw_scores = {:#?}", raw_scores);
        eprintln!("filtered_scores = {:#?}", filtered_scores);
        println!("{:#?}", it.elapsed());
    }
}
