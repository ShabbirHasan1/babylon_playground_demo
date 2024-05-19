use anyhow::Result;
use std::{
    borrow::Borrow,
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use itertools::{Itertools, MinMaxResult};
use ndarray::{arr1, arr2, Array1, Array2};
use ordered_float::OrderedFloat;
use parking_lot::RwLock;
use serde_json::Value;
use yata::{
    core::{Action, PeriodType, ValueType},
    methods::{Highest, Lowest, MeanAbsDev, MedianAbsDev, ReversalSignal, StDev, HMA, ROC, SMA},
    prelude::*,
};

use crate::{
    appraiser::Appraiser,
    orderbook_level::OrderBookLevel,
    types::{NormalizedOrderType, NormalizedSide, OrderedValueType},
    util::{calc_relative_position_between, round_float_to_ordered_precision, round_float_to_precision},
};
use ndarray_stats::*;
use ustr::Ustr;

pub const SIZED_ORDERBOOK_LENGTH: usize = 20;

/// TODO: Orderbook must support grouping by price functionality, I'm sure you know what that is, but if you don't please let me know before we proceed.
/// TODO: Orderbook must have a way to limit the length of each side to prevent uncontrollable growth in size but this is optional.
///         Let's say that we limit by a price range, such that positions whose price is outside of the range are ignored
///             and don't make it into the orderbook. This is optional.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct OrderBookTrade {
    pub side:              NormalizedSide,
    pub quote_or_quantity: OrderedValueType,
    pub anchor_position:   usize,
    pub anchor_level:      OrderBookLevel,
    pub better_price:      Option<OrderedValueType>,
}

impl OrderBookTrade {
    pub fn new(
        side: NormalizedSide,
        quote_or_quantity: OrderedValueType,
        anchor_position: usize,
        anchor_level: OrderBookLevel,
        better_price: Option<OrderedValueType>,
    ) -> Self {
        Self {
            side,
            quote_or_quantity,
            anchor_position,
            anchor_level,
            better_price,
        }
    }

    pub fn compute(&self, appraiser: &Appraiser) -> (OrderedValueType, OrderedValueType, bool) {
        match self.side {
            NormalizedSide::Buy => match self.better_price {
                None => (self.anchor_level.price, OrderedFloat(appraiser.normalize_base(self.quote_or_quantity.0 / self.anchor_level.price.0)), false),
                Some(better_price) => (
                    OrderedFloat(appraiser.normalize_quote(better_price.0)),
                    OrderedFloat(appraiser.normalize_base(self.quote_or_quantity.0 / better_price.0)),
                    true,
                ),
            },
            NormalizedSide::Sell => match self.better_price {
                None => (self.anchor_level.price, OrderedFloat(appraiser.normalize_base(self.quote_or_quantity.0 * self.anchor_level.price.0)), false),
                Some(better_price) => (
                    OrderedFloat(appraiser.normalize_quote(better_price.0)),
                    OrderedFloat(appraiser.normalize_base(self.quote_or_quantity.0 * better_price.0)),
                    true,
                ),
            },
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Suggestion {
    pub entry_side: NormalizedSide,
    pub entry:      OrderBookTrade,
    pub exit:       OrderBookTrade,
    pub stop:       OrderBookTrade,
    pub stop_limit: Option<OrderBookTrade>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct RawSuggestion {
    pub entry_side:              NormalizedSide,
    pub entry_price:             OrderedValueType,
    pub entry_quote_or_quantity: OrderedValueType,
    pub entry_return:            OrderedValueType,
    pub exit_price:              OrderedValueType,
    pub stop_price:              OrderedValueType,
    pub stop_limit:              Option<OrderedValueType>,
}

impl Suggestion {
    pub fn as_raw(&self, a: &Appraiser) -> RawSuggestion {
        let entry_price = self.entry.better_price.unwrap_or(self.entry.anchor_level.price);
        let entry_quote_or_quantity = self.entry.quote_or_quantity;
        let entry_return = OrderedFloat(a.normalize_base(entry_quote_or_quantity.0 / entry_price.0));

        let exit_price = OrderedFloat(a.normalize_quote(self.exit.better_price.unwrap_or(self.exit.anchor_level.price).0));
        let stop_price = OrderedFloat(a.normalize_quote(self.stop.better_price.unwrap_or(self.stop.anchor_level.price).0));

        RawSuggestion {
            entry_side: self.entry_side,
            entry_price,
            entry_quote_or_quantity,
            entry_return,
            exit_price,
            stop_price,
            stop_limit: None,
        }
    }

    pub fn entry(&self) -> (OrderedValueType, OrderedValueType, OrderedValueType) {
        let entry_price = self.entry.better_price.unwrap_or(self.entry.anchor_level.price);
        let posted_amount = self.entry.quote_or_quantity;
        let expected_return = posted_amount / entry_price;
        (entry_price, posted_amount, expected_return)
    }

    pub fn exit(&self) -> [OrderedValueType; 2] {
        let entry = self.entry();
        [self.exit.better_price.unwrap_or(self.exit.anchor_level.price), entry.1 / entry.0]
    }

    pub fn stop(&self) -> [OrderedValueType; 2] {
        let entry = self.entry();
        [self.stop.better_price.unwrap_or(self.stop.anchor_level.price), entry.1 / entry.0]
    }
}

#[derive(Debug, Clone, Default)]
pub struct OrderBookSummary {
    pub market_price:          OrderedValueType,
    pub bid_best:              OrderBookLevel,
    pub bid_edge:              OrderBookLevel,
    pub ask_best:              OrderBookLevel,
    pub ask_edge:              OrderBookLevel,
    pub spread:                OrderedValueType,
    pub spread_pc:             OrderedValueType,
    pub range:                 OrderedValueType,
    pub range_pc:              OrderedValueType,
    pub bid_depth:             OrderedValueType,
    pub bid_range:             OrderedValueType,
    pub ask_depth:             OrderedValueType,
    pub ask_range:             OrderedValueType,
    pub snd_ratio:             OrderedValueType,
    pub snd_ratio_sma:         OrderedValueType,
    pub snd_ratio_sma_highest: OrderedValueType,
    pub snd_ratio_sma_lowest:  OrderedValueType,
    pub snd_ratio_sma_loc:     OrderedValueType,
    pub bid_quantity_mean:     OrderedValueType,
    pub ask_quantity_mean:     OrderedValueType,
    pub min_bid_quantity:      (usize, OrderBookLevel),
    pub max_bid_quantity:      (usize, OrderBookLevel),
    pub min_ask_quantity:      (usize, OrderBookLevel),
    pub max_ask_quantity:      (usize, OrderBookLevel),
    pub compute_time:          Duration,
}

#[derive(Clone)]
pub struct OrderBook(pub Arc<RwLock<OrderBookInner>>);

impl OrderBook {
    pub fn new(name: Ustr, appraiser: Appraiser, last_update_id: u64, bids: Vec<[OrderedValueType; 2]>, asks: Vec<[OrderedValueType; 2]>) -> Self {
        let mut inner = OrderBookInner {
            name,
            appraiser,
            last_update_id: 0,
            bids: Default::default(),
            asks: Default::default(),
            base_precision: appraiser.true_base_precision as usize,
            quote_precision: appraiser.true_quote_precision as usize,
            summary: Default::default(),
            snd_ratio_sma: SMA::new(50, &0.0).unwrap(),
            snd_ratio_sma_highest: Highest::new(600, &-100.0).unwrap(),
            snd_ratio_sma_lowest: Lowest::new(600, &100.0).unwrap(),
            price: OrderedFloat(0.0),
        };

        inner.update_diff(OrderedFloat(0.0), last_update_id, bids, asks);

        Self(Arc::new(RwLock::new(inner)))
    }

    pub fn new_partial_20(name: Ustr, appraiser: Appraiser, last_update_id: u64, bids: [[OrderedValueType; 2]; 20], asks: [[OrderedValueType; 2]; 20]) -> Self {
        let mut inner = OrderBookInner {
            name,
            appraiser,
            last_update_id: 0,
            bids: Default::default(),
            asks: Default::default(),
            base_precision: appraiser.true_base_precision as usize,
            quote_precision: appraiser.true_quote_precision as usize,
            summary: Default::default(),
            snd_ratio_sma: SMA::new(50, &0.0).unwrap(),
            snd_ratio_sma_highest: Highest::new(600, &-100.0).unwrap(),
            snd_ratio_sma_lowest: Lowest::new(600, &100.0).unwrap(),
            price: OrderedFloat(0.0),
        };

        inner.update_partial_20(OrderedFloat(0.0), last_update_id, bids, asks);

        Self(Arc::new(RwLock::new(inner)))
    }
}

#[derive(Clone, Debug)]
pub struct OrderBookInner {
    name:                  Ustr,
    price:                 OrderedValueType,
    bids:                  [OrderBookLevel; SIZED_ORDERBOOK_LENGTH],
    asks:                  [OrderBookLevel; SIZED_ORDERBOOK_LENGTH],
    snd_ratio_sma:         SMA,
    snd_ratio_sma_highest: Highest,
    snd_ratio_sma_lowest:  Lowest,
    pub appraiser:         Appraiser,
    pub last_update_id:    u64,
    pub summary:           OrderBookSummary,
    pub base_precision:    usize,
    pub quote_precision:   usize,
}

impl OrderBookInner {
    pub fn round_quote_to_precision(&self, quote: ValueType) -> ValueType {
        self.appraiser.round_quote(quote)
    }

    pub fn round_base_to_precision(&self, base: ValueType) -> ValueType {
        self.appraiser.round_base(base)
    }

    pub fn update_partial_20(
        &mut self,
        market_price: OrderedValueType,
        last_update_id: u64,
        bids: [[OrderedValueType; 2]; 20],
        asks: [[OrderedValueType; 2]; 20],
    ) -> Result<()> {
        if self.last_update_id >= last_update_id {
            return Ok(());
        }

        // minmax positions
        self.summary.min_bid_quantity = (0, OrderBookLevel::new(bids[0][0], bids[0][1]));
        self.summary.max_bid_quantity = (0, OrderBookLevel::new(bids[0][0], bids[0][1]));
        self.summary.min_ask_quantity = (0, OrderBookLevel::new(asks[0][0], asks[0][1]));
        self.summary.max_ask_quantity = (0, OrderBookLevel::new(asks[0][0], asks[0][1]));

        // ----------------------------------------------------------------------------
        // updating positions

        for (i, [price, quantity]) in bids.into_iter().enumerate() {
            self.bids[i].price = OrderedFloat(self.appraiser.round_quote(price.0));
            self.bids[i].quantity = OrderedFloat(self.appraiser.round_base(quantity.0));

            self.summary.min_bid_quantity = match self.summary.min_bid_quantity.1.quantity.cmp(&quantity) {
                Ordering::Greater => (i, self.bids[i]),
                Ordering::Equal if self.summary.min_bid_quantity.0 > i => (i, self.bids[i]),
                _ => self.summary.min_bid_quantity,
            };

            self.summary.max_bid_quantity = match self.summary.max_bid_quantity.1.quantity.cmp(&quantity) {
                Ordering::Less => (i, self.bids[i]),
                Ordering::Equal if self.summary.max_bid_quantity.0 > i => (i, self.bids[i]),
                _ => self.summary.max_bid_quantity,
            };
        }

        for (i, [price, quantity]) in asks.into_iter().enumerate() {
            self.asks[i].price = OrderedFloat(self.appraiser.round_quote(price.0));
            self.asks[i].quantity = OrderedFloat(self.appraiser.round_base(quantity.0));

            self.summary.min_ask_quantity = match self.summary.min_ask_quantity.1.quantity.cmp(&quantity) {
                Ordering::Greater => (i, self.asks[i]),
                Ordering::Equal if self.summary.min_ask_quantity.0 > i => (i, self.asks[i]),
                _ => self.summary.min_ask_quantity,
            };

            self.summary.max_ask_quantity = match self.summary.max_ask_quantity.1.quantity.cmp(&quantity) {
                Ordering::Less => (i, self.asks[i]),
                Ordering::Equal if self.summary.max_ask_quantity.0 > i => (i, self.asks[i]),
                _ => self.summary.max_ask_quantity,
            };
        }

        self.last_update_id = last_update_id;

        // ----------------------------------------------------------------------------
        // computing summary

        let it = Instant::now();

        // bid
        let bid_best = self.bids[0];
        let bid_edge = self.bids[SIZED_ORDERBOOK_LENGTH - 1];
        let bid_range = bid_best.price - bid_edge.price;
        let bid_depth = self.bids.into_iter().fold(OrderedFloat(0.0 as ValueType), |acc, x| acc + x.quantity);

        // ask
        let ask_best = self.asks[0];
        let ask_edge = self.asks[SIZED_ORDERBOOK_LENGTH - 1];
        let ask_range = ask_edge.price - ask_best.price;
        let ask_depth = self.asks.into_iter().fold(OrderedFloat(0.0 as ValueType), |acc, x| acc + x.quantity);

        // ask-bid spread
        let spread = ask_best.price - bid_best.price;
        let spread_pc = spread / 100.0;

        // ask-bid range
        let range = ask_edge.price - bid_edge.price;
        let range_pc = range / 100.0;

        // supply and demand

        let snd_ratio = ask_depth / bid_depth;
        let snd_ratio_sma = OrderedFloat(self.snd_ratio_sma.next(&snd_ratio));
        let snd_ratio_sma_highest = match snd_ratio_sma.is_infinite() {
            true => -100.0,
            false => self.snd_ratio_sma_highest.next(&snd_ratio_sma),
        };
        let snd_ratio_sma_lowest = match snd_ratio_sma.is_infinite() {
            true => 100.0,
            false => self.snd_ratio_sma_lowest.next(&snd_ratio_sma),
        };
        let snd_ratio_sma_loc = calc_relative_position_between(snd_ratio_sma.0, snd_ratio_sma_lowest, snd_ratio_sma_highest);

        // TODO: compute average quantity but ignore spikes

        let bid_quantity_mean = self.bids.into_iter().fold(OrderedFloat(0.0), |acc, x| acc + x.quantity) / SIZED_ORDERBOOK_LENGTH as ValueType;
        let ask_quantity_mean = self.asks.into_iter().fold(OrderedFloat(0.0), |acc, x| acc + x.quantity) / SIZED_ORDERBOOK_LENGTH as ValueType;

        self.summary = OrderBookSummary {
            market_price,
            bid_best,
            bid_edge,
            ask_best,
            ask_edge,
            spread: self.round_quote_to_precision(spread.0).into(),
            spread_pc: round_float_to_ordered_precision(spread_pc.0, 4),
            range: self.round_quote_to_precision(range.0).into(),
            range_pc: round_float_to_ordered_precision(range_pc.0, 4),
            bid_depth: self.round_base_to_precision(bid_depth.0).into(),
            bid_range: self.round_quote_to_precision(bid_range.0).into(),
            ask_depth: self.round_base_to_precision(ask_depth.0).into(),
            ask_range: self.round_quote_to_precision(ask_range.0).into(),
            snd_ratio: round_float_to_ordered_precision(snd_ratio.into(), 2),
            snd_ratio_sma: round_float_to_ordered_precision(snd_ratio_sma.into(), 2),
            snd_ratio_sma_highest: round_float_to_ordered_precision(snd_ratio_sma_highest.into(), 2),
            snd_ratio_sma_lowest: round_float_to_ordered_precision(snd_ratio_sma_lowest.into(), 2),
            snd_ratio_sma_loc: round_float_to_ordered_precision(snd_ratio_sma_loc.into(), 2),
            bid_quantity_mean,
            ask_quantity_mean,
            compute_time: it.elapsed(),
            ..self.summary
        };

        Ok(())
    }

    pub fn update_diff(
        &mut self,
        market_price: OrderedValueType,
        last_update_id: u64,
        bids: Vec<[OrderedValueType; 2]>,
        asks: Vec<[OrderedValueType; 2]>,
    ) -> Result<()> {
        if self.last_update_id >= last_update_id {
            return Ok(());
        }

        // minmax positions
        self.summary.min_bid_quantity = (0, OrderBookLevel::new(bids[0][0], bids[0][1]));
        self.summary.max_bid_quantity = (0, OrderBookLevel::new(bids[0][0], bids[0][1]));
        self.summary.min_ask_quantity = (0, OrderBookLevel::new(asks[0][0], asks[0][1]));
        self.summary.max_ask_quantity = (0, OrderBookLevel::new(asks[0][0], asks[0][1]));

        // ----------------------------------------------------------------------------
        // updating positions

        for (i, [price, quantity]) in bids.into_iter().enumerate() {
            self.bids[i].price = OrderedFloat(self.appraiser.round_quote(price.0));
            self.bids[i].quantity = OrderedFloat(self.appraiser.round_base(quantity.0));

            self.summary.min_bid_quantity = match self.summary.min_bid_quantity.1.quantity.cmp(&quantity) {
                Ordering::Greater => (i, self.bids[i]),
                Ordering::Equal if self.summary.min_bid_quantity.0 > i => (i, self.bids[i]),
                _ => self.summary.min_bid_quantity,
            };

            self.summary.max_bid_quantity = match self.summary.max_bid_quantity.1.quantity.cmp(&quantity) {
                Ordering::Less => (i, self.bids[i]),
                Ordering::Equal if self.summary.max_bid_quantity.0 > i => (i, self.bids[i]),
                _ => self.summary.max_bid_quantity,
            };
        }

        for (i, [price, quantity]) in asks.into_iter().enumerate() {
            self.asks[i].price = OrderedFloat(self.appraiser.round_quote(price.0));
            self.asks[i].quantity = OrderedFloat(self.appraiser.round_base(quantity.0));

            self.summary.min_ask_quantity = match self.summary.min_ask_quantity.1.quantity.cmp(&quantity) {
                Ordering::Greater => (i, self.asks[i]),
                Ordering::Equal if self.summary.min_ask_quantity.0 > i => (i, self.asks[i]),
                _ => self.summary.min_ask_quantity,
            };

            self.summary.max_ask_quantity = match self.summary.max_ask_quantity.1.quantity.cmp(&quantity) {
                Ordering::Less => (i, self.asks[i]),
                Ordering::Equal if self.summary.max_ask_quantity.0 > i => (i, self.asks[i]),
                _ => self.summary.max_ask_quantity,
            };
        }

        self.last_update_id = last_update_id;

        // ----------------------------------------------------------------------------
        // computing summary

        let it = Instant::now();

        // bid
        let bid_best = self.bids[0];
        let bid_edge = self.bids[SIZED_ORDERBOOK_LENGTH - 1];
        let bid_range = bid_best.price - bid_edge.price;
        let bid_depth = self.bids.into_iter().fold(OrderedFloat(0.0 as ValueType), |acc, x| acc + x.quantity);

        // ask
        let ask_best = self.asks[0];
        let ask_edge = self.asks[SIZED_ORDERBOOK_LENGTH - 1];
        let ask_range = ask_edge.price - ask_best.price;
        let ask_depth = self.asks.into_iter().fold(OrderedFloat(0.0 as ValueType), |acc, x| acc + x.quantity);

        // ask-bid spread
        let spread = ask_best.price - bid_best.price;
        let spread_pc = spread / 100.0;

        // ask-bid range
        let range = ask_edge.price - bid_edge.price;
        let range_pc = range / 100.0;

        // supply and demand

        let snd_ratio = ask_depth / bid_depth;
        let snd_ratio_sma = OrderedFloat(self.snd_ratio_sma.next(&snd_ratio));
        let snd_ratio_sma_highest = match snd_ratio_sma.is_infinite() {
            true => -100.0,
            false => self.snd_ratio_sma_highest.next(&snd_ratio_sma),
        };
        let snd_ratio_sma_lowest = match snd_ratio_sma.is_infinite() {
            true => 100.0,
            false => self.snd_ratio_sma_lowest.next(&snd_ratio_sma),
        };
        let snd_ratio_sma_loc = calc_relative_position_between(snd_ratio_sma.0, snd_ratio_sma_lowest, snd_ratio_sma_highest);

        // TODO: compute average quantity but ignore spikes

        let bid_quantity_mean = self.bids.into_iter().fold(OrderedFloat(0.0), |acc, x| acc + x.quantity) / SIZED_ORDERBOOK_LENGTH as ValueType;
        let ask_quantity_mean = self.asks.into_iter().fold(OrderedFloat(0.0), |acc, x| acc + x.quantity) / SIZED_ORDERBOOK_LENGTH as ValueType;

        self.summary = OrderBookSummary {
            market_price,
            bid_best,
            bid_edge,
            ask_best,
            ask_edge,
            spread: self.round_quote_to_precision(spread.0).into(),
            spread_pc: round_float_to_ordered_precision(spread_pc.0, 4),
            range: self.round_quote_to_precision(range.0).into(),
            range_pc: round_float_to_ordered_precision(range_pc.0, 4),
            bid_depth: self.round_base_to_precision(bid_depth.0).into(),
            bid_range: self.round_quote_to_precision(bid_range.0).into(),
            ask_depth: self.round_base_to_precision(ask_depth.0).into(),
            ask_range: self.round_quote_to_precision(ask_range.0).into(),
            snd_ratio: round_float_to_ordered_precision(snd_ratio.into(), 2),
            snd_ratio_sma: round_float_to_ordered_precision(snd_ratio_sma.into(), 2),
            snd_ratio_sma_highest: round_float_to_ordered_precision(snd_ratio_sma_highest.into(), 2),
            snd_ratio_sma_lowest: round_float_to_ordered_precision(snd_ratio_sma_lowest.into(), 2),
            snd_ratio_sma_loc: round_float_to_ordered_precision(snd_ratio_sma_loc.into(), 2),
            bid_quantity_mean,
            ask_quantity_mean,
            compute_time: it.elapsed(),
            ..self.summary
        };

        Ok(())
    }

    pub fn walls(&self, side: NormalizedSide, vicinity_threshold: OrderedValueType) -> Vec<(usize, OrderBookLevel)> {
        let mut walls = Vec::with_capacity(8);
        let mut prev_max = self.max_quantity(side, 0..SIZED_ORDERBOOK_LENGTH);
        walls.push(prev_max);

        let mean_quantity = match side {
            NormalizedSide::Buy => self.summary.bid_quantity_mean,
            NormalizedSide::Sell => self.summary.ask_quantity_mean,
        };

        loop {
            let next_max = self.max_quantity(side, 0..prev_max.0);
            let r = next_max.1.quantity / prev_max.1.quantity;

            if prev_max.0 == next_max.0 || r >= vicinity_threshold || next_max.1.quantity <= mean_quantity {
                return walls;
            } else {
                prev_max = next_max;
                walls.push(next_max);
            }
        }
    }

    pub fn min_quantity(&self, side: NormalizedSide, range: Range<usize>) -> (usize, OrderBookLevel) {
        let side = match side {
            NormalizedSide::Buy => &self.bids,
            NormalizedSide::Sell => &self.asks,
        };

        match side.get(range) {
            None => (0, OrderBookLevel::default()),
            Some(range) => range
                .into_iter()
                .enumerate()
                .fold((0, side[0]), |acc, (i, lvl)| match acc.1.quantity.cmp(&lvl.quantity) {
                    Ordering::Greater => (i, *lvl),
                    Ordering::Equal if acc.0 > i => (i, *lvl),
                    _ => acc,
                }),
        }
    }

    pub fn min_quantity_in(&self, slice: &[(usize, OrderBookLevel)], range: Range<usize>) -> (usize, OrderBookLevel) {
        match slice.get(range) {
            None => slice[0],
            Some(range) => range.into_iter().fold(slice[0], |acc, (i, lvl)| match acc.1.quantity.cmp(&lvl.quantity) {
                Ordering::Greater => (*i, *lvl),
                Ordering::Equal if acc.0 > *i => (*i, *lvl),
                _ => acc,
            }),
        }
    }

    pub fn max_quantity(&self, side: NormalizedSide, range: Range<usize>) -> (usize, OrderBookLevel) {
        let side = match side {
            NormalizedSide::Buy => &self.bids,
            NormalizedSide::Sell => &self.asks,
        };

        match side.get(range) {
            None => (0, OrderBookLevel::default()),
            Some(range) => range
                .into_iter()
                .enumerate()
                .fold((0, side[0]), |acc, (i, lvl)| match acc.1.quantity.cmp(&lvl.quantity) {
                    Ordering::Less => (i, *lvl),
                    Ordering::Equal if acc.0 > i => (i, *lvl),
                    _ => acc,
                }),
        }
    }

    pub fn max_quantity_in(&self, slice: &[(usize, OrderBookLevel)], range: Range<usize>) -> (usize, OrderBookLevel) {
        match slice.get(range) {
            None => slice[0],
            Some(range) => range.into_iter().fold(slice[0], |acc, (i, lvl)| match acc.1.quantity.cmp(&lvl.quantity) {
                Ordering::Less => (*i, *lvl),
                Ordering::Equal if acc.0 > *i => (*i, *lvl),
                _ => acc,
            }),
        }
    }

    pub fn bids(&self) -> &[OrderBookLevel; SIZED_ORDERBOOK_LENGTH] {
        &self.bids
    }

    pub fn asks(&self) -> &[OrderBookLevel; SIZED_ORDERBOOK_LENGTH] {
        &self.asks
    }

    pub fn undercut_ask_level_price(&self, pos: usize) -> Option<OrderedValueType> {
        if pos == 0 {
            return None;
        }

        if let [Some((prev_i, prev_lvl)), Some((actual_i, actual_lvl))] = self.level_with_previous(NormalizedSide::Sell, pos) {
            let rules = &self.appraiser.rules.price_filter.expect("price_filter");
            let undercut_price = (actual_lvl.price - rules.tick_size);

            if undercut_price >= prev_lvl.price {
                Some(undercut_price)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn frontrun_ask_level_price(&self, pos: usize) -> Option<OrderedValueType> {
        if pos == 0 {
            return None;
        }

        if let [Some((actual_i, actual_lvl)), Some((next_i, next_lvl))] = self.level_with_next(NormalizedSide::Sell, pos) {
            let rules = &self.appraiser.rules.price_filter.expect("price_filter");
            let undercut_price = (actual_lvl.price + rules.tick_size);

            if undercut_price <= next_lvl.price {
                Some(undercut_price)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn frontrun_bid_level_price(&self, pos: usize) -> Option<OrderedValueType> {
        if pos == 0 {
            return None;
        }

        if let [Some((prev_i, prev_lvl)), Some((actual_i, actual_lvl))] = self.level_with_previous(NormalizedSide::Buy, pos) {
            let rules = &self.appraiser.rules.price_filter.expect("price_filter");
            let upper_price = OrderedFloat(self.round_quote_to_precision((actual_lvl.price + rules.tick_size).0));

            if upper_price <= prev_lvl.price {
                Some(upper_price)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn undercut_bid_level_price(&self, pos: usize) -> Option<OrderedValueType> {
        if pos == 0 {
            return None;
        }

        if let [Some((actual_i, actual_lvl)), Some((next_i, next_lvl))] = self.level_with_next(NormalizedSide::Buy, pos) {
            let rules = &self.appraiser.rules.price_filter.expect("price_filter");
            let undercut_price = (actual_lvl.price - rules.tick_size);

            if undercut_price >= next_lvl.price {
                Some(undercut_price)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn best_fit_for_bid_quote_by_fit_factor(&self, quote: OrderedValueType, fit_factor: OrderedValueType) -> Option<OrderBookTrade> {
        match self.bids.into_iter().find_position(|lvl| (lvl.quantity * lvl.price) / quote >= fit_factor) {
            None => return None,
            Some(original_suggestion @ (original_pos, original_lvl)) => Some(match self.frontrun_bid_level_price(original_pos) {
                None => OrderBookTrade::new(NormalizedSide::Buy, quote, original_pos, original_lvl, None),
                Some(better_price) => OrderBookTrade::new(NormalizedSide::Buy, quote, original_pos, original_lvl, Some(better_price)),
            }),
        }
    }

    pub fn entry_suggestions_quote(&self, quote: OrderedValueType, vicinity_threshold: OrderedValueType) -> Vec<OrderBookTrade> {
        self.walls(NormalizedSide::Buy, vicinity_threshold)
            .into_iter()
            .map(|(i, lvl)| match self.frontrun_bid_level_price(i) {
                None => OrderBookTrade::new(NormalizedSide::Buy, quote, i, lvl, None),
                Some(better_price) => OrderBookTrade::new(NormalizedSide::Buy, quote, i, lvl, Some(better_price)),
            })
            .collect()
    }

    pub fn exit_suggestions_qty(&self, quantity: OrderedValueType, vicinity_threshold: OrderedValueType) -> Vec<OrderBookTrade> {
        self.walls(NormalizedSide::Sell, vicinity_threshold)
            .into_iter()
            .map(|(i, lvl)| match self.undercut_ask_level_price(i) {
                None => OrderBookTrade::new(NormalizedSide::Sell, quantity, i, lvl, None),
                Some(better_price) => OrderBookTrade::new(NormalizedSide::Sell, quantity, i, lvl, Some(better_price)),
            })
            .collect()
    }

    pub fn best_fit_for_ask_by_fit_factor(&self, quantity: OrderedValueType, fit_factor: OrderedValueType) -> Option<OrderBookTrade> {
        match self.asks.into_iter().find_position(|lvl| (lvl.quantity / quantity) >= fit_factor) {
            None => return None,
            Some(original_suggestion @ (original_pos, original_lvl)) => Some(match self.undercut_ask_level_price(original_pos) {
                None => OrderBookTrade::new(NormalizedSide::Sell, quantity, original_pos, original_lvl, None),
                Some(better_price) => OrderBookTrade::new(NormalizedSide::Sell, quantity, original_pos, original_lvl, Some(better_price)),
            }),
        }
    }

    fn level_with_previous(&self, side: NormalizedSide, pos: usize) -> [Option<(usize, &OrderBookLevel)>; 2] {
        let side = match side {
            NormalizedSide::Buy => self.bids(),
            NormalizedSide::Sell => self.asks(),
        };

        let previous = match side.get(pos - 1) {
            None => None,
            Some(lvl) => Some((pos - 1, lvl)),
        };

        let actual = match side.get(pos) {
            None => None,
            Some(lvl) => Some((pos, lvl)),
        };

        [previous, actual]
    }

    fn level_with_next(&self, side: NormalizedSide, pos: usize) -> [Option<(usize, &OrderBookLevel)>; 2] {
        let side = match side {
            NormalizedSide::Buy => self.bids(),
            NormalizedSide::Sell => self.asks(),
        };

        let actual = match side.get(pos) {
            None => None,
            Some(lvl) => Some((pos, lvl)),
        };

        let next = match side.get(pos + 1) {
            None => None,
            Some(lvl) => Some((pos + 1, lvl)),
        };

        [actual, next]
    }

    fn level_with_neighbors(&self, side: NormalizedSide, pos: usize) -> [Option<(usize, &OrderBookLevel)>; 3] {
        let side = match side {
            NormalizedSide::Buy => self.bids(),
            NormalizedSide::Sell => self.asks(),
        };

        let previous = match side.get(pos - 1) {
            None => None,
            Some(lvl) => Some((pos - 1, lvl)),
        };

        let actual = match side.get(pos) {
            None => None,
            Some(lvl) => Some((pos, lvl)),
        };

        let next_after = match side.get(pos + 1) {
            None => None,
            Some(lvl) => Some((pos + 1, lvl)),
        };

        [previous, actual, next_after]
    }

    pub fn suggested_long_roundtrip_quote(
        &self,
        quote: OrderedValueType,
        entry_fit_factor: OrderedValueType,
        entry_vicinity_threshold: OrderedValueType,
        exit_vicinity_threshold: OrderedValueType,
    ) -> Option<Suggestion> {
        let Some(&entry_wall) = self.walls(NormalizedSide::Buy, entry_vicinity_threshold).first() else {
            return None;
        };
        let Some(&exit_wall) = self.walls(NormalizedSide::Sell, exit_vicinity_threshold).first() else {
            return None;
        };

        let entry = match self.best_fit_for_bid_quote_by_fit_factor(quote, entry_fit_factor) {
            None => return None,
            Some(entry) => entry,
        };

        let exit = match self.undercut_ask_level_price(exit_wall.0) {
            None => OrderBookTrade::new(NormalizedSide::Sell, exit_wall.1.quantity, exit_wall.0, exit_wall.1, None),
            Some(better_price) => OrderBookTrade::new(NormalizedSide::Sell, exit_wall.1.quantity, exit_wall.0, exit_wall.1, Some(better_price)),
        };

        // finding biggest observable wall
        // WARNING: position index is actually on the BID side for displaying only
        let expected_entry_return = entry.compute(&self.appraiser).1;
        let stop = match self.undercut_bid_level_price(entry_wall.0) {
            None => OrderBookTrade::new(NormalizedSide::Buy, expected_entry_return, entry_wall.0, entry_wall.1, None),
            Some(better_price) => OrderBookTrade::new(NormalizedSide::Buy, expected_entry_return, entry_wall.0, entry_wall.1, Some(better_price)),
        };

        Some(Suggestion {
            entry_side: NormalizedSide::Buy,
            entry,
            exit,
            stop,
            stop_limit: None,
        })
    }

    /*
    pub fn suggested_long_roundtrip_quote(&self, quote: OrderedValueType, fit_factor: OrderedValueType) -> Option<SuggestedRoundtrip> {
        let entry = match self.best_fit_for_bid_quote(quote, fit_factor) {
            None => return None,
            Some(entry) => entry,
        };

        let exit = match self.best_fit_for_ask(entry.price(&self.appraiser).1, fit_factor) {
            None => return None,
            Some(exit) => exit,
        };

        Some(SuggestedRoundtrip {
            entry_side: Side::Buy,
            entry,
            exit,
            stop: None,
            stop_limit: None,
        })
    }
     */

    /*
    fn suggested_long_roundtrip_qty(&self, qty: OrderedValueType, fit_factor: OrderedValueType) -> Option<SuggestedRoundtrip> {
        // TODO: implement undercutting if there is space between best fit and the previous level

        let best_fit_for_bid = self.best_fit_for_bid_qty(qty, fit_factor);
        let best_fit_for_ask = match best_fit_for_bid {
            None => None,
            Some((i, level)) => self.best_fit_for_ask(level.qty, fit_factor),
        };

        match (best_fit_for_bid, best_fit_for_ask) {
            (Some(entry), Some(exit)) => Some(SuggestedRoundtrip {
                entry_side: Side::Buy,
                entry,
                exit,
                stop: None,
                stop_limit: None,
            }),
            _ => None,
        }
    }
     */
}
