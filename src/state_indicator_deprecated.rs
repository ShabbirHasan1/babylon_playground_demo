use crate::{
    candle::Candle,
    config::{CoinConfig, TimeframeConfig},
    // indicators::correlation_cycle::CorrelationCycle,
    support_resistance::SRLevel,
    timeframe::Timeframe,
    timeset::TimeSet,
};
use anyhow::{bail, Result};
use itertools::Itertools;
use log::info;
use ordered_float::OrderedFloat;
use std::time::Duration;
use yata::{
    core::{IndicatorConfig, IndicatorInstance, IndicatorResult, Method, MovingAverageConstructor, PeriodType, Source, ValueType, Window},
    helpers::{MAInstance, MA},
    indicators::{
        AverageDirectionalIndex, AwesomeOscillator, ChaikinMoneyFlow, CommodityChannelIndex, EaseOfMovement, EldersForceIndex, IchimokuCloud, Kaufman,
        KeltnerChannel, KnowSureThing, MomentumIndex, MoneyFlowIndex, ParabolicSAR, PivotReversalStrategy, PriceChannelStrategy, RelativeStrengthIndex,
        RelativeVigorIndex, SMIErgodicIndicator, StochasticOscillator, TrendStrengthIndex, Trix, TrueStrengthIndex, KAMA, MACD,
    },
    methods::{Highest, HighestLowestDelta, LinReg, LinearVolatility, Lowest, Momentum, StDev, ADI, EMA, LSMA, ROC, SMA, TR},
};

#[derive(Clone)]
pub struct FrozenIndicatorState {
    pub tr:              TR,
    pub hld_20_close:    HighestLowestDelta,
    pub highest_20_high: Highest,
    pub highest_20_low:  Highest,
    pub lowest_20_high:  Lowest,
    pub lowest_20_low:   Lowest,
    pub ci_rsi:          <RelativeStrengthIndex as IndicatorConfig>::Instance,
    pub ci_rsi_sma:      MAInstance,
    pub ci_rsi_aux:      <RelativeStrengthIndex as IndicatorConfig>::Instance,
    pub ci_rsi_mom:      Momentum,
    pub ci_ma_fast:      MAInstance,
    pub ci_ma_medium:    MAInstance,
    pub ci_ma_slow:      MAInstance,
    pub mom_3:           Momentum,
    pub mom_5:           Momentum,
    pub mom_8:           Momentum,
    pub mom_13:          Momentum,
    pub mom_21:          Momentum,
    pub mom_34:          Momentum,
    pub adi_3:           ADI,
    pub adi_5:           ADI,
    pub adi_8:           ADI,
    pub adi_13:          ADI,
    pub adi_21:          ADI,
    pub adi_34:          ADI,
    pub atr_3:           MAInstance,
    pub atr_5:           MAInstance,
    pub atr_8:           MAInstance,
    pub atr_13:          MAInstance,
    pub atr_21:          MAInstance,
    pub atr_34:          MAInstance,
    pub stdev_3:         StDev,
    pub stdev_5:         StDev,
    pub stdev_8:         StDev,
    pub stdev_13:        StDev,
    pub stdev_21:        StDev,
    pub stdev_34:        StDev,
    pub linvol_3:        LinearVolatility,
    pub linvol_5:        LinearVolatility,
    pub linvol_8:        LinearVolatility,
    pub linvol_13:       LinearVolatility,
    pub linvol_21:       LinearVolatility,
    pub linvol_34:       LinearVolatility,
    pub linreg_3:        LinReg,
    pub linreg_5:        LinReg,
    pub linreg_8:        LinReg,
    pub linreg_13:       LinReg,
    pub linreg_21:       LinReg,
    pub linreg_34:       LinReg,
    pub rsi_14:          <RelativeStrengthIndex as IndicatorConfig>::Instance,
    pub rsi_14_wma_14:   MAInstance,
    pub cci_18:          <CommodityChannelIndex as IndicatorConfig>::Instance,
    pub cci_18_wma_18:   MAInstance,
    pub stoch:           <StochasticOscillator as IndicatorConfig>::Instance,
    pub macd:            <MACD as IndicatorConfig>::Instance,
    pub kst:             <KnowSureThing as IndicatorConfig>::Instance,
    pub cmf:             <ChaikinMoneyFlow as IndicatorConfig>::Instance,
    pub mfi:             <MoneyFlowIndex as IndicatorConfig>::Instance,
    pub kama:            <Kaufman as IndicatorConfig>::Instance,
    pub prs:             <PivotReversalStrategy as IndicatorConfig>::Instance,
    pub efi:             <EldersForceIndex as IndicatorConfig>::Instance,
    pub psar:            <ParabolicSAR as IndicatorConfig>::Instance,
    pub adx:             <AverageDirectionalIndex as IndicatorConfig>::Instance,
    pub trix:            <Trix as IndicatorConfig>::Instance,
    pub trsi:            <TrendStrengthIndex as IndicatorConfig>::Instance,
    pub tsi:             <TrueStrengthIndex as IndicatorConfig>::Instance,
    pub rvi:             <RelativeVigorIndex as IndicatorConfig>::Instance,
    pub momi:            <MomentumIndex as IndicatorConfig>::Instance,
    pub awesome:         <AwesomeOscillator as IndicatorConfig>::Instance,
    pub pcs:             <PriceChannelStrategy as IndicatorConfig>::Instance,
    pub eom:             <EaseOfMovement as IndicatorConfig>::Instance,
    pub smiergo:         <SMIErgodicIndicator as IndicatorConfig>::Instance,
    // pub corci:           <CorrelationCycle as IndicatorConfig>::Instance,
}

impl FrozenIndicatorState {
    pub fn new(
        timeframe_config: &TimeframeConfig,
        computed_state_window_size: PeriodType,
        trailing_candles: &Vec<Candle>,
    ) -> Result<(Self, Window<ComputedIndicatorState>, ComputedIndicatorState)> {
        let first_candle = trailing_candles.first().expect("trailing_candles is empty");
        let second_candle = trailing_candles.iter().nth(1).cloned().expect("failed to get second candle");
        let third_candle = trailing_candles.iter().nth(2).cloned().expect("failed to get third candle");

        // ----------------------------------------------------------------------------
        //  Constance Brown Composite Index (inspired by LazyBear on TradingView)
        // ----------------------------------------------------------------------------

        let ci_rsi_length = 14;
        let ci_rsi_mom_length = 9;
        let ci_rsi_ma_length = 3;
        let ci_ma_length = 3;
        let ci_fast_length = 3;
        let ci_medium_length = 13;
        let ci_slow_length = 33;

        let mut ci_rsi = RelativeStrengthIndex {
            ma:            MA::RMA(ci_rsi_length),
            smooth_ma:     MA::EMA(ci_rsi_length),
            primary_zone:  0.20,
            source:        Source::Close,
            smoothed_zone: 0.30,
        }
        .init(first_candle)
        .unwrap();

        let mut ci_rsi_aux = RelativeStrengthIndex {
            ma:            MA::RMA(ci_rsi_ma_length),
            smooth_ma:     MA::EMA(ci_rsi_ma_length),
            primary_zone:  0.20,
            source:        Source::Close,
            smoothed_zone: 0.30,
        }
        .init(first_candle)
        .unwrap();

        // ----------------------------------------------------------------------------
        // WARNING: complex mixing, work with great care
        // ----------------------------------------------------------------------------

        let ci_rsi_result_2nd_candle = ci_rsi.next(&second_candle);
        let ci_rsi_aux_result_2nd_candle = ci_rsi_aux.next(&second_candle);
        let ci_rsi_result_3rd_candle = ci_rsi.next(&third_candle);
        let ci_rsi_aux_result_3rd_candle = ci_rsi_aux.next(&third_candle);

        let ci_rsi_value_2nd_candle = ci_rsi_result_2nd_candle.value(0);
        let ci_rsi_aux_value_2nd_candle = ci_rsi_aux_result_2nd_candle.value(0);
        let ci_rsi_value_3rd_candle = ci_rsi_result_3rd_candle.value(0);
        let ci_rsi_aux_value_3rd_candle = ci_rsi_aux_result_3rd_candle.value(0);

        // RSI SMA
        let mut ci_rsi_sma = MA::SMA(ci_rsi_length).init(ci_rsi_value_2nd_candle)?;

        // CI
        let mut ci_rsi_mom = Momentum::new(ci_rsi_mom_length, &ci_rsi_value_2nd_candle)?;
        let mut ci_rsi_sma = MA::SMA(ci_ma_length).init(ci_rsi_aux_value_2nd_candle)?;
        let ci_rsi_mom_value = ci_rsi_mom.next(&ci_rsi_value_3rd_candle);
        let ci_rsi_sma_value = ci_rsi_sma.next(&ci_rsi_aux_value_3rd_candle);
        let ci = ci_rsi_mom_value + ci_rsi_sma_value;
        let mut ci_ma_fast = MA::SMA(ci_fast_length).init(ci)?;
        let mut ci_ma_medium = MA::SMA(ci_medium_length).init(ci)?;
        let mut ci_ma_slow = MA::SMA(ci_slow_length).init(ci)?;

        // ----------------------------------------------------------------------------
        // RSI
        // ----------------------------------------------------------------------------

        let mut rsi_14 = RelativeStrengthIndex {
            ma:            MA::RMA(14),
            smooth_ma:     MA::EMA(14),
            primary_zone:  0.20,
            source:        Source::Close,
            smoothed_zone: 0.30,
        }
        .init(first_candle)
        .unwrap();

        let rsi_14_value = rsi_14.next(&second_candle);
        let mut rsi_14_wma_14 = MA::WMA(14).init(rsi_14_value.value(0))?;
        let rsi_14_value = rsi_14.next(&third_candle);
        let rsi_14_wma_14_value = rsi_14_wma_14.next(&rsi_14_value.value(0));

        // ----------------------------------------------------------------------------
        // CCI
        // ----------------------------------------------------------------------------

        let mut cci_18 = CommodityChannelIndex {
            period: 18,
            zone:   1.0,
            source: Source::Close,
        }
        .init(first_candle)
        .unwrap();

        let cci_18_value = cci_18.next(first_candle);
        let mut cci_18_wma_18 = MA::WMA(18).init(cci_18_value.value(0))?;

        let cci_18_value = cci_18.next(&second_candle);
        let cci_18_wma_18_value = cci_18_wma_18.next(&cci_18_value.value(0));

        // ----------------------------------------------------------------------------
        // Stochastic
        // ----------------------------------------------------------------------------

        let mut stoch = StochasticOscillator::default().init(first_candle).unwrap();
        stoch.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Correlation Cycle Indicator
        // ----------------------------------------------------------------------------

        // let mut corci = CorrelationCycle::default().init(first_candle).unwrap();
        // corci.next(&second_candle);

        // ----------------------------------------------------------------------------
        // TRIX
        // ----------------------------------------------------------------------------

        let mut trix = Trix::default().init(first_candle).unwrap();
        trix.next(&second_candle);

        // ----------------------------------------------------------------------------
        // True Strength Index
        // ----------------------------------------------------------------------------

        let mut tsi = TrueStrengthIndex::default().init(first_candle).unwrap();
        tsi.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Trend Strength Index
        // ----------------------------------------------------------------------------

        let mut trsi = TrendStrengthIndex::default().init(first_candle).unwrap();
        trsi.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Relative Vigor Index
        // ----------------------------------------------------------------------------

        let mut rvi = RelativeVigorIndex::default().init(first_candle).unwrap();
        rvi.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Momentum Index
        // ----------------------------------------------------------------------------

        let mut momi = MomentumIndex::default().init(first_candle).unwrap();
        momi.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Awesome Oscillator
        // ----------------------------------------------------------------------------

        let mut awesome = AwesomeOscillator::default().init(first_candle).unwrap();
        awesome.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Price Channel Strategy
        // ----------------------------------------------------------------------------

        let mut pcs = PriceChannelStrategy::default().init(first_candle).unwrap();
        pcs.next(&second_candle);

        // ----------------------------------------------------------------------------
        // SMI Ergodic Indicator
        // ----------------------------------------------------------------------------

        let mut smiergo = SMIErgodicIndicator::default().init(first_candle).unwrap();
        smiergo.next(&second_candle);

        // ----------------------------------------------------------------------------
        // MACD
        // ----------------------------------------------------------------------------

        let mut macd = MACD::default().init(first_candle).unwrap();
        macd.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Pivot Reversal Strategy
        // ----------------------------------------------------------------------------

        let mut prs = PivotReversalStrategy::default().init(first_candle).unwrap();
        prs.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Parabolic SAR
        // ----------------------------------------------------------------------------

        let mut psar = ParabolicSAR { af_step: 0.022, af_max: 0.2 }.init(first_candle).unwrap();
        psar.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Average Directional Index
        // ----------------------------------------------------------------------------

        let mut adx = AverageDirectionalIndex::default().init(first_candle).unwrap();
        adx.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Elder's Force Index
        // ----------------------------------------------------------------------------

        let mut efi = EldersForceIndex {
            ma:      MA::EMA(13),
            period2: 1,
            source:  Source::Close,
        }
        .init(first_candle)
        .unwrap();
        efi.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Kaufman
        // ----------------------------------------------------------------------------

        let mut kama = Kaufman::default().init(first_candle)?;
        kama.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Know Sure Thing
        // ----------------------------------------------------------------------------

        let mut kst = KnowSureThing::default().init(first_candle).unwrap();
        kst.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Chaikin Money Flow
        // ----------------------------------------------------------------------------

        let mut cmf = ChaikinMoneyFlow::default().init(first_candle).unwrap();
        cmf.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Ease of Movement
        // ----------------------------------------------------------------------------

        let mut eom = EaseOfMovement::default().init(first_candle).unwrap();
        eom.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Money Flow Index
        // ----------------------------------------------------------------------------

        let mut mfi = MoneyFlowIndex::default().init(first_candle).unwrap();
        mfi.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Highs and Lows
        // ----------------------------------------------------------------------------

        let mut hld_20_close = HighestLowestDelta::new(20, &first_candle.close).unwrap();
        hld_20_close.next(&second_candle.close);

        let mut highest_20_high = Highest::new(20, &first_candle.high).unwrap();
        highest_20_high.next(&second_candle.high);

        let mut highest_20_low = Highest::new(20, &first_candle.low).unwrap();
        highest_20_low.next(&second_candle.low);

        let mut lowest_20_high = Lowest::new(20, &first_candle.high).unwrap();
        lowest_20_high.next(&second_candle.high);

        let mut lowest_20_low = Lowest::new(20, &first_candle.low).unwrap();
        lowest_20_low.next(&second_candle.low);

        // ----------------------------------------------------------------------------
        // Linear Volatility
        // ----------------------------------------------------------------------------

        let mut linvol_3 = LinearVolatility::new(3, &first_candle.close).unwrap();
        linvol_3.next(&second_candle.close);

        let mut linvol_5 = LinearVolatility::new(5, &first_candle.close).unwrap();
        linvol_5.next(&second_candle.close);

        let mut linvol_8 = LinearVolatility::new(8, &first_candle.close).unwrap();
        linvol_8.next(&second_candle.close);

        let mut linvol_13 = LinearVolatility::new(13, &first_candle.close).unwrap();
        linvol_13.next(&second_candle.close);

        let mut linvol_21 = LinearVolatility::new(21, &first_candle.close).unwrap();
        linvol_21.next(&second_candle.close);

        let mut linvol_34 = LinearVolatility::new(34, &first_candle.close).unwrap();
        linvol_34.next(&second_candle.close);

        // ----------------------------------------------------------------------------
        // Linear Regression
        // ----------------------------------------------------------------------------

        let mut linreg_3 = LinReg::new(3, &first_candle.close).unwrap();
        linreg_3.next(&second_candle.close);

        let mut linreg_5 = LinReg::new(5, &first_candle.close).unwrap();
        linreg_5.next(&second_candle.close);

        let mut linreg_8 = LinReg::new(8, &first_candle.close).unwrap();
        linreg_8.next(&second_candle.close);

        let mut linreg_13 = LinReg::new(13, &first_candle.close).unwrap();
        linreg_13.next(&second_candle.close);

        let mut linreg_21 = LinReg::new(21, &first_candle.close).unwrap();
        linreg_21.next(&second_candle.close);

        let mut linreg_34 = LinReg::new(34, &first_candle.close).unwrap();
        linreg_34.next(&second_candle.close);

        // ----------------------------------------------------------------------------
        // Accumulation Distribution Index
        // ----------------------------------------------------------------------------

        let mut adi_3 = ADI::new(3, first_candle).unwrap();
        adi_3.next(&second_candle);

        let mut adi_5 = ADI::new(5, first_candle).unwrap();
        adi_5.next(&second_candle);

        let mut adi_8 = ADI::new(8, first_candle).unwrap();
        adi_8.next(&second_candle);

        let mut adi_13 = ADI::new(13, first_candle).unwrap();
        adi_13.next(&second_candle);

        let mut adi_21 = ADI::new(21, first_candle).unwrap();
        adi_21.next(&second_candle);

        let mut adi_34 = ADI::new(34, first_candle).unwrap();
        adi_34.next(&second_candle);

        // ----------------------------------------------------------------------------
        // Momentum
        // ----------------------------------------------------------------------------

        let mut mom_3 = Momentum::new(3, &first_candle.close).unwrap();
        mom_3.next(&second_candle.close);

        let mut mom_5 = Momentum::new(5, &first_candle.close).unwrap();
        mom_5.next(&second_candle.close);

        let mut mom_8 = Momentum::new(8, &first_candle.close).unwrap();
        mom_8.next(&second_candle.close);

        let mut mom_13 = Momentum::new(13, &first_candle.close).unwrap();
        mom_13.next(&second_candle.close);

        let mut mom_21 = Momentum::new(21, &first_candle.close).unwrap();
        mom_21.next(&second_candle.close);

        let mut mom_34 = Momentum::new(34, &first_candle.close).unwrap();
        mom_34.next(&second_candle.close);

        // ----------------------------------------------------------------------------
        // Standard Deviation
        // ----------------------------------------------------------------------------

        let mut stdev_3 = StDev::new(3, &first_candle.close).unwrap();
        stdev_3.next(&second_candle.close);

        let mut stdev_5 = StDev::new(5, &first_candle.close).unwrap();
        stdev_5.next(&second_candle.close);

        let mut stdev_8 = StDev::new(8, &first_candle.close).unwrap();
        stdev_8.next(&second_candle.close);

        let mut stdev_13 = StDev::new(13, &first_candle.close).unwrap();
        stdev_13.next(&second_candle.close);

        let mut stdev_21 = StDev::new(21, &first_candle.close).unwrap();
        stdev_21.next(&second_candle.close);

        let mut stdev_34 = StDev::new(34, &first_candle.close).unwrap();
        stdev_34.next(&second_candle.close);

        // ----------------------------------------------------------------------------
        // Average True Range
        // ----------------------------------------------------------------------------

        let mut tr = TR::new(first_candle).unwrap();
        let tr_value = tr.next(&second_candle);

        let mut atr_3 = MA::EMA(3).init(tr_value)?;
        let mut atr_5 = MA::EMA(5).init(tr_value)?;
        let mut atr_8 = MA::EMA(8).init(tr_value)?;
        let mut atr_13 = MA::EMA(13).init(tr_value)?;
        let mut atr_21 = MA::EMA(21).init(tr_value)?;
        let mut atr_34 = MA::EMA(34).init(tr_value)?;

        let tr_value = tr.next(&third_candle);

        // ----------------------------------------------------------------------------
        // building the window
        // ----------------------------------------------------------------------------

        let first_state = ComputedIndicatorState {
            candle:                 first_candle.clone(),
            hld_20_close:           hld_20_close.next(&third_candle.close),
            highest_20_high:        highest_20_high.next(&third_candle.high),
            highest_20_low:         highest_20_low.next(&third_candle.low),
            lowest_20_high:         lowest_20_high.next(&third_candle.high),
            lowest_20_low:          lowest_20_low.next(&third_candle.low),
            tr:                     tr_value,
            mom_3:                  mom_3.next(&third_candle.close),
            mom_5:                  mom_5.next(&third_candle.close),
            mom_8:                  mom_8.next(&third_candle.close),
            mom_13:                 mom_13.next(&third_candle.close),
            mom_21:                 mom_21.next(&third_candle.close),
            mom_34:                 mom_34.next(&third_candle.close),
            adi_3:                  adi_3.next(&third_candle),
            adi_5:                  adi_5.next(&third_candle),
            adi_8:                  adi_8.next(&third_candle),
            adi_13:                 adi_13.next(&third_candle),
            adi_21:                 adi_21.next(&third_candle),
            adi_34:                 adi_34.next(&third_candle),
            stdev_3:                stdev_3.next(&third_candle.close),
            stdev_5:                stdev_5.next(&third_candle.close),
            stdev_8:                stdev_8.next(&third_candle.close),
            stdev_13:               stdev_13.next(&third_candle.close),
            stdev_21:               stdev_21.next(&third_candle.close),
            stdev_34:               stdev_34.next(&third_candle.close),
            atr_3:                  atr_3.next(&tr_value),
            atr_5:                  atr_5.next(&tr_value),
            atr_8:                  atr_8.next(&tr_value),
            atr_13:                 atr_13.next(&tr_value),
            atr_21:                 atr_21.next(&tr_value),
            atr_34:                 atr_34.next(&tr_value),
            linvol_3:               linvol_3.next(&third_candle.close),
            linvol_5:               linvol_5.next(&third_candle.close),
            linvol_8:               linvol_8.next(&third_candle.close),
            linvol_13:              linvol_13.next(&third_candle.close),
            linvol_21:              linvol_21.next(&third_candle.close),
            linvol_34:              linvol_34.next(&third_candle.close),
            linreg_3:               linreg_3.next(&third_candle.close),
            linreg_3_tan:           linreg_3.tan(),
            linreg_5:               linreg_5.next(&third_candle.close),
            linreg_5_tan:           linreg_5.tan(),
            linreg_8:               linreg_8.next(&third_candle.close),
            linreg_8_tan:           linreg_8.tan(),
            linreg_13:              linreg_13.next(&third_candle.close),
            linreg_13_tan:          linreg_13.tan(),
            linreg_21:              linreg_21.next(&third_candle.close),
            linreg_21_tan:          linreg_21.tan(),
            linreg_34:              linreg_34.next(&third_candle.close),
            linreg_34_tan:          linreg_34.tan(),
            rsi_14:                 rsi_14_value,
            rsi_14_wma_14:          rsi_14_wma_14_value,
            macd:                   macd.next(&third_candle),
            cmf:                    cmf.next(&third_candle),
            mfi:                    mfi.next(&third_candle),
            kst:                    kst.next(&third_candle),
            kama:                   kama.next(&third_candle),
            prs:                    prs.next(&third_candle),
            efi:                    efi.next(&third_candle),
            psar:                   psar.next(&third_candle),
            adx:                    adx.next(&third_candle),
            cci_18:                 cci_18.next(&third_candle),
            cci_18_wma_18:          cci_18_wma_18_value,
            stoch:                  stoch.next(&third_candle),
            trix:                   trix.next(&third_candle),
            trsi:                   trsi.next(&third_candle),
            tsi:                    tsi.next(&third_candle),
            rvi:                    rvi.next(&third_candle),
            momi:                   momi.next(&third_candle),
            awesome:                awesome.next(&third_candle),
            pcs:                    pcs.next(&third_candle),
            smiergo:                smiergo.next(&third_candle),
            // corci:                  corci.next(&third_candle),
            total_computation_time: Default::default(),
        };

        // initializing state window and filling in the first value
        let mut states = Window::new(computed_state_window_size, first_state);
        let num_candles = trailing_candles.len();

        // NOTE: redundant atm, but will be used in the future
        let mut prev_candle = third_candle;

        // NOTE: keeping last state to become the head state at the end of this computation
        let mut last_state = first_state;

        trailing_candles.iter().enumerate().skip(3).for_each(|(i, c)| {
            // ----------------------------------------------------------------------------
            // RSI
            // ----------------------------------------------------------------------------

            let rsi_14_value = rsi_14.next(c);
            let rsi_14_wma_14_value = rsi_14_wma_14.next(&rsi_14_value.value(0));

            // ----------------------------------------------------------------------------
            // Commodity Channel Index
            // ----------------------------------------------------------------------------

            let cci_18_value = cci_18.next(c);
            let cci_18_wma_18_value = cci_18_wma_18.next(&cci_18_value.value(0));

            // ----------------------------------------------------------------------------
            // Linear Volatility
            // ----------------------------------------------------------------------------

            let mut linvol_3_value = linvol_3.next(&c.close);
            let mut linvol_5_value = linvol_5.next(&c.close);
            let mut linvol_8_value = linvol_8.next(&c.close);
            let mut linvol_13_value = linvol_13.next(&c.close);
            let mut linvol_21_value = linvol_21.next(&c.close);
            let mut linvol_34_value = linvol_34.next(&c.close);

            // ----------------------------------------------------------------------------
            // Linear Regression
            // ----------------------------------------------------------------------------

            let mut linreg_3_value = linreg_3.next(&c.close);
            let mut linreg_3_tan_value = linreg_3.tan();
            let mut linreg_5_value = linreg_5.next(&c.close);
            let mut linreg_5_tan_value = linreg_5.tan();
            let mut linreg_8_value = linreg_8.next(&c.close);
            let mut linreg_8_tan_value = linreg_8.tan();
            let mut linreg_13_value = linreg_13.next(&c.close);
            let mut linreg_13_tan_value = linreg_13.tan();
            let mut linreg_21_value = linreg_21.next(&c.close);
            let mut linreg_21_tan_value = linreg_21.tan();
            let mut linreg_34_value = linreg_34.next(&c.close);
            let mut linreg_34_tan_value = linreg_34.tan();

            // ----------------------------------------------------------------------------
            // Momentum
            // ----------------------------------------------------------------------------

            let mut mom_3_value = mom_3.next(&c.close);
            let mut mom_5_value = mom_5.next(&c.close);
            let mut mom_8_value = mom_8.next(&c.close);
            let mut mom_13_value = mom_13.next(&c.close);
            let mut mom_21_value = mom_21.next(&c.close);
            let mut mom_34_value = mom_34.next(&c.close);

            // ----------------------------------------------------------------------------
            // Accumulation Distribution Index
            // ----------------------------------------------------------------------------

            let mut adi_3_value = adi_3.next(c);
            let mut adi_5_value = adi_5.next(c);
            let mut adi_8_value = adi_8.next(c);
            let mut adi_13_value = adi_13.next(c);
            let mut adi_21_value = adi_21.next(c);
            let mut adi_34_value = adi_34.next(c);

            // ----------------------------------------------------------------------------
            // Standard Deviation
            // ----------------------------------------------------------------------------

            let mut stdev_3_value = stdev_3.next(&c.close);
            let mut stdev_5_value = stdev_5.next(&c.close);
            let mut stdev_8_value = stdev_8.next(&c.close);
            let mut stdev_13_value = stdev_13.next(&c.close);
            let mut stdev_21_value = stdev_21.next(&c.close);
            let mut stdev_34_value = stdev_34.next(&c.close);

            // ----------------------------------------------------------------------------
            // Average True Range
            // ----------------------------------------------------------------------------

            let mut atr_3_value = atr_3.next(&c.close);
            let mut atr_5_value = atr_5.next(&c.close);
            let mut atr_8_value = atr_8.next(&c.close);
            let mut atr_13_value = atr_13.next(&c.close);
            let mut atr_21_value = atr_21.next(&c.close);
            let mut atr_34_value = atr_34.next(&c.close);

            // ----------------------------------------------------------------------------
            // Correlation Cycle Indicator
            // ----------------------------------------------------------------------------

            // ----------------------------------------------------------------------------
            // Misc
            // NOTE: here goes something that isn't worth categorizing separately
            // ----------------------------------------------------------------------------

            // TODO: ... if anything

            // ----------------------------------------------------------------------------
            // pushing in historical states
            // ----------------------------------------------------------------------------
            let computed_state = ComputedIndicatorState {
                candle:                 c.clone(),
                hld_20_close:           hld_20_close.next(&c.close),
                highest_20_high:        highest_20_high.next(&c.high),
                highest_20_low:         highest_20_low.next(&c.low),
                lowest_20_high:         lowest_20_high.next(&c.high),
                lowest_20_low:          lowest_20_low.next(&c.low),
                tr:                     tr.next(c),
                mom_3:                  mom_3_value,
                mom_5:                  mom_5_value,
                mom_8:                  mom_8_value,
                mom_13:                 mom_13_value,
                mom_21:                 mom_21_value,
                mom_34:                 mom_34_value,
                adi_3:                  adi_3_value,
                adi_5:                  adi_5_value,
                adi_8:                  adi_8_value,
                adi_13:                 adi_13_value,
                adi_21:                 adi_21_value,
                adi_34:                 adi_34_value,
                stdev_3:                stdev_3_value,
                stdev_5:                stdev_5_value,
                stdev_8:                stdev_8_value,
                stdev_13:               stdev_13_value,
                stdev_21:               stdev_21_value,
                stdev_34:               stdev_34_value,
                atr_3:                  atr_3_value,
                atr_5:                  atr_5_value,
                atr_8:                  atr_8_value,
                atr_13:                 atr_13_value,
                atr_21:                 atr_21_value,
                atr_34:                 atr_34_value,
                linvol_3:               linvol_3_value,
                linvol_5:               linvol_5_value,
                linvol_8:               linvol_8_value,
                linvol_13:              linvol_13_value,
                linvol_21:              linvol_21_value,
                linvol_34:              linvol_34_value,
                linreg_3:               linreg_3_value,
                linreg_3_tan:           linreg_3_tan_value,
                linreg_5:               linreg_5_value,
                linreg_5_tan:           linreg_5_tan_value,
                linreg_8:               linreg_8_value,
                linreg_8_tan:           linreg_8_tan_value,
                linreg_13:              linreg_13_value,
                linreg_13_tan:          linreg_13_tan_value,
                linreg_21:              linreg_21_value,
                linreg_21_tan:          linreg_21_tan_value,
                linreg_34:              linreg_34_value,
                linreg_34_tan:          linreg_34_tan_value,
                rsi_14:                 rsi_14_value,
                rsi_14_wma_14:          rsi_14_wma_14_value,
                macd:                   macd.next(c),
                cmf:                    cmf.next(c),
                mfi:                    mfi.next(c),
                kst:                    kst.next(c),
                kama:                   kama.next(c),
                prs:                    prs.next(c),
                efi:                    efi.next(c),
                psar:                   psar.next(c),
                adx:                    adx.next(c),
                cci_18:                 cci_18.next(c),
                cci_18_wma_18:          cci_18_wma_18_value,
                stoch:                  stoch.next(c),
                trix:                   trix.next(c),
                trsi:                   trsi.next(c),
                tsi:                    tsi.next(c),
                rvi:                    rvi.next(c),
                momi:                   momi.next(c),
                awesome:                awesome.next(c),
                pcs:                    pcs.next(c),
                smiergo:                smiergo.next(c),
                // corci:                  corci.next(c),
                total_computation_time: Default::default(),
            };

            // preserving candle for just in case
            prev_candle = *c;

            if i < num_candles - 1 {
                states.push(computed_state);
            } else {
                // NOTE: Preserving latest state to later become the head state
                last_state = computed_state;
            }
        });

        let frozen_state = Self {
            tr,
            hld_20_close,
            highest_20_high,
            highest_20_low,
            lowest_20_high,
            lowest_20_low,
            ci_rsi,
            ci_rsi_sma,
            ci_rsi_aux,
            ci_rsi_mom,
            ci_ma_fast,
            ci_ma_medium,
            ci_ma_slow,
            mom_3,
            mom_5,
            mom_8,
            mom_13,
            mom_21,
            mom_34,
            adi_3,
            adi_5,
            adi_8,
            adi_13,
            adi_21,
            adi_34,
            atr_3,
            atr_5,
            atr_8,
            atr_13,
            atr_21,
            atr_34,
            stdev_3,
            stdev_5,
            stdev_8,
            stdev_13,
            stdev_21,
            stdev_34,
            linvol_3,
            linvol_5,
            linvol_8,
            linvol_13,
            linvol_21,
            linvol_34,
            linreg_3,
            linreg_5,
            linreg_8,
            linreg_13,
            linreg_21,
            linreg_34,
            rsi_14,
            rsi_14_wma_14,
            cci_18,
            cci_18_wma_18,
            stoch,
            macd,
            kst,
            cmf,
            mfi,
            prs,
            efi,
            psar,
            adx,
            trix,
            trsi,
            tsi,
            rvi,
            momi,
            awesome,
            pcs,
            eom,
            kama,
            smiergo,
            // corci,
        };

        Ok((frozen_state, states, last_state))
    }
}

#[derive(Copy, Clone, Debug)]
pub struct ComputedIndicatorState {
    pub candle:                 Candle,
    pub hld_20_close:           ValueType,
    pub highest_20_high:        ValueType,
    pub highest_20_low:         ValueType,
    pub lowest_20_high:         ValueType,
    pub lowest_20_low:          ValueType,
    pub tr:                     ValueType,
    pub mom_3:                  ValueType,
    pub mom_5:                  ValueType,
    pub mom_8:                  ValueType,
    pub mom_13:                 ValueType,
    pub mom_21:                 ValueType,
    pub mom_34:                 ValueType,
    pub adi_3:                  ValueType,
    pub adi_5:                  ValueType,
    pub adi_8:                  ValueType,
    pub adi_13:                 ValueType,
    pub adi_21:                 ValueType,
    pub adi_34:                 ValueType,
    pub stdev_3:                ValueType,
    pub stdev_5:                ValueType,
    pub stdev_8:                ValueType,
    pub stdev_13:               ValueType,
    pub stdev_21:               ValueType,
    pub stdev_34:               ValueType,
    pub atr_3:                  ValueType,
    pub atr_5:                  ValueType,
    pub atr_8:                  ValueType,
    pub atr_13:                 ValueType,
    pub atr_21:                 ValueType,
    pub atr_34:                 ValueType,
    pub linvol_3:               ValueType,
    pub linvol_5:               ValueType,
    pub linvol_8:               ValueType,
    pub linvol_13:              ValueType,
    pub linvol_21:              ValueType,
    pub linvol_34:              ValueType,
    pub linreg_3:               ValueType,
    pub linreg_3_tan:           ValueType,
    pub linreg_5:               ValueType,
    pub linreg_5_tan:           ValueType,
    pub linreg_8:               ValueType,
    pub linreg_8_tan:           ValueType,
    pub linreg_13:              ValueType,
    pub linreg_13_tan:          ValueType,
    pub linreg_21:              ValueType,
    pub linreg_21_tan:          ValueType,
    pub linreg_34:              ValueType,
    pub linreg_34_tan:          ValueType,
    pub rsi_14:                 IndicatorResult,
    pub rsi_14_wma_14:          ValueType,
    pub cmf:                    IndicatorResult,
    pub mfi:                    IndicatorResult,
    pub kst:                    IndicatorResult,
    pub macd:                   IndicatorResult,
    pub kama:                   IndicatorResult,
    pub prs:                    IndicatorResult,
    pub efi:                    IndicatorResult,
    pub psar:                   IndicatorResult,
    pub adx:                    IndicatorResult,
    pub cci_18:                 IndicatorResult,
    pub cci_18_wma_18:          ValueType,
    pub stoch:                  IndicatorResult,
    pub trix:                   IndicatorResult,
    pub trsi:                   IndicatorResult,
    pub tsi:                    IndicatorResult,
    pub rvi:                    IndicatorResult,
    pub momi:                   IndicatorResult,
    pub awesome:                IndicatorResult,
    pub pcs:                    IndicatorResult,
    pub smiergo:                IndicatorResult,
    // pub corci:                  IndicatorResult,
    pub total_computation_time: Duration,
}

impl Default for ComputedIndicatorState {
    fn default() -> Self {
        Self {
            candle:                 Default::default(),
            hld_20_close:           0.0,
            highest_20_high:        0.0,
            highest_20_low:         0.0,
            lowest_20_high:         0.0,
            lowest_20_low:          0.0,
            tr:                     0.0,
            mom_3:                  0.0,
            mom_5:                  0.0,
            mom_8:                  0.0,
            mom_13:                 0.0,
            mom_21:                 0.0,
            mom_34:                 0.0,
            adi_3:                  0.0,
            adi_5:                  0.0,
            adi_8:                  0.0,
            adi_13:                 0.0,
            adi_21:                 0.0,
            adi_34:                 0.0,
            stdev_3:                0.0,
            stdev_5:                0.0,
            stdev_8:                0.0,
            stdev_13:               0.0,
            stdev_21:               0.0,
            stdev_34:               0.0,
            atr_3:                  0.0,
            atr_5:                  0.0,
            atr_8:                  0.0,
            atr_13:                 0.0,
            atr_21:                 0.0,
            atr_34:                 0.0,
            linvol_3:               0.0,
            linvol_5:               0.0,
            linvol_8:               0.0,
            linvol_13:              0.0,
            linvol_21:              0.0,
            linvol_34:              0.0,
            linreg_3:               0.0,
            linreg_3_tan:           0.0,
            linreg_5:               0.0,
            linreg_5_tan:           0.0,
            linreg_8:               0.0,
            linreg_8_tan:           0.0,
            linreg_13:              0.0,
            linreg_13_tan:          0.0,
            linreg_21:              0.0,
            linreg_21_tan:          0.0,
            linreg_34:              0.0,
            linreg_34_tan:          0.0,
            rsi_14:                 Default::default(),
            rsi_14_wma_14:          0.0,
            cmf:                    Default::default(),
            mfi:                    Default::default(),
            kst:                    Default::default(),
            kama:                   Default::default(),
            prs:                    Default::default(),
            efi:                    Default::default(),
            psar:                   Default::default(),
            adx:                    Default::default(),
            cci_18:                 Default::default(),
            cci_18_wma_18:          0.0,
            stoch:                  Default::default(),
            trix:                   Default::default(),
            trsi:                   Default::default(),
            tsi:                    Default::default(),
            rvi:                    Default::default(),
            momi:                   Default::default(),
            awesome:                Default::default(),
            pcs:                    Default::default(),
            macd:                   Default::default(),
            smiergo:                Default::default(),
            // corci:                  Default::default(),
            total_computation_time: Default::default(),
        }
    }
}
