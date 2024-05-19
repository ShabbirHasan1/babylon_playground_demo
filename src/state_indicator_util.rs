use crate::candle::Candle;
use anyhow::{bail, Result};
use itertools::Itertools;
use yata::{
    core::{IndicatorConfig, IndicatorInstance, MovingAverageConstructor, PeriodType, Source, ValueType},
    helpers::{MAInstance, MA},
    indicators::RelativeStrengthIndex,
    prelude::*,
};

pub fn init_rsi_with_moving_averages(
    rsi_period: PeriodType,
    mas: &[MA],
    initial_candles: [Candle; 2],
    zone: ValueType,
    source: Source,
) -> Result<(<RelativeStrengthIndex as IndicatorConfig>::Instance, Vec<MAInstance>)> {
    let [first_candle, second_candle] = initial_candles;

    let mut rsi = RelativeStrengthIndex {
        ma: MA::RMA(rsi_period),
        smooth_ma: MA::RMA(rsi_period),
        primary_zone: 0.20,
        smoothed_zone: 0.30,
        source,
    }
    .init(&first_candle)?;

    let mut rsi_result = rsi.next(&second_candle);

    let mut initialized_mas = Vec::new();

    for ma in mas {
        initialized_mas.push(match ma {
            ma @ MA::SMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::WMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::HMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::RMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::EMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::DMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::DEMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::TMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::TEMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::WSMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::SMM(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::SWMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::TRIMA(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::LinReg(period) => ma.init(rsi_result.value(0))?,
            ma @ MA::Vidya(period) => ma.init(rsi_result.value(0))?,

            _ => bail!("unsupported moving average"),
        });
    }

    Ok((rsi, initialized_mas))
}
