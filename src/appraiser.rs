use crate::{
    f,
    model::Rules,
    types::{Lot, NormalizedOrderType, OrderedValueType},
    util::{round_float_to_precision, s2f},
};
use num_traits::Zero;
use ordered_float::OrderedFloat;
use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use ustr::Ustr;
use yata::core::ValueType;

#[derive(Copy, Clone, Debug)]
pub struct Appraiser {
    pub rules:                Rules,
    pub true_base_precision:  i32,
    pub true_quote_precision: i32,
}

impl Appraiser {
    pub fn new(rules: Rules) -> Self {
        let mut true_base_precision = 2;
        let mut true_quote_precision = 2;

        let quantity_step_size = rules.lot_size.map(|x| x.step_size).expect("quantity_step_size");
        let price_tick_size = rules.price_filter.map(|x| x.tick_size).expect("price_tick_size");

        // base precision
        let mut x = quantity_step_size;
        while x < f!(0.01000000) {
            x /= f!(0.1);
            true_base_precision += 1;
        }

        // quote precision
        let mut x = price_tick_size;
        while x < f!(0.01000000) {
            x /= f!(0.1);
            true_quote_precision += 1;
        }

        Self {
            rules,
            true_base_precision,
            true_quote_precision,
        }
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn compute_quantity(&self, mut quote: ValueType, mut price: ValueType) -> ValueType {
        self.normalize_base(quote / self.normalize_quote(price))
    }

    pub fn compute_bid_lot(&self, mut quote_to_spend: ValueType, mut price: ValueType, mut trigger_price: Option<ValueType>) -> Lot {
        quote_to_spend = self.normalize_quote(quote_to_spend);
        price = self.normalize_quote(price);
        trigger_price = trigger_price.map(|trigger_price| self.normalize_quote(trigger_price));

        Lot {
            price,
            quantity: self.normalize_base(quote_to_spend / price),
            trigger_price,
        }
    }

    pub fn compute_ask_lot(&self, mut base_to_spend: ValueType, mut price: ValueType, mut trigger_price: Option<ValueType>) -> Lot {
        price = self.normalize_quote(price);
        trigger_price = trigger_price.map(|trigger_price| self.normalize_quote(trigger_price));

        Lot {
            price,
            quantity: self.normalize_base(base_to_spend),
            trigger_price,
        }
    }

    pub fn is_valid_price(&self, price: ValueType) -> bool {
        let price = f!(price);
        let rules = &self.rules.price_filter.expect("price_filter");

        let a = if rules.min_price.is_zero() { true } else { price >= rules.min_price };
        let b = if rules.max_price.is_zero() { true } else { price <= rules.max_price };
        let c = if rules.tick_size.is_zero() { true } else { ((price - rules.min_price) % rules.tick_size).is_zero() };

        a && b && c
    }

    pub fn is_valid_quantity(&self, quantity: ValueType) -> bool {
        let quantity = f!(quantity);
        let rules = &self.rules.lot_size.expect("lot_size");

        let a = if rules.min_qty.is_zero() { true } else { quantity >= rules.min_qty };
        let b = if rules.max_qty.is_zero() { true } else { quantity <= rules.max_qty };
        let c = if rules.step_size.is_zero() { true } else { ((quantity - rules.min_qty) % rules.step_size).is_zero() };

        a && b && c
    }

    pub fn is_dust_quantity(&self, quantity: ValueType) -> bool {
        let rules = &self.rules.lot_size.expect("lot_size");
        f!(quantity) < rules.min_qty
    }

    pub fn price_up_percent(&self, price: ValueType, percent: ValueType) -> ValueType {
        self.normalize_quote(price * (1.0 + percent.abs()))
    }

    pub fn price_down_percent(&self, price: ValueType, percent: ValueType) -> ValueType {
        self.normalize_quote(price * (1.0 - percent.abs()))
    }

    pub fn price_oneup(&self, price: ValueType) -> ValueType {
        let rules = &self.rules.price_filter.expect("price_filter");
        self.normalize_quote(price) + rules.tick_size.0
    }

    pub fn price_undercut(&self, price: ValueType) -> ValueType {
        let rules = &self.rules.price_filter.expect("price_filter");
        self.normalize_quote(price) - rules.tick_size.0
    }

    pub fn normalize_quote<F: Into<ValueType>>(&self, price: F) -> ValueType {
        let price = price.into();
        let rules = &self.rules.price_filter.expect("price_filter");
        let tick_remainder = price % rules.tick_size.0;
        self.round_quote(if tick_remainder.is_zero() { price } else { price - (price % rules.tick_size.0) })
    }

    pub fn normalize_base(&self, base: ValueType) -> ValueType {
        let rules = &self.rules.lot_size.expect("lot_size");
        let step_remainder = base % rules.step_size.0;
        self.round_base(if step_remainder.is_zero() { base } else { base - step_remainder })
    }

    pub fn round_quote(&self, quote: ValueType) -> ValueType {
        round_float_to_precision(quote, self.true_quote_precision)
    }

    pub fn round_normalized_quote(&self, quote: ValueType) -> ValueType {
        round_float_to_precision(self.normalize_quote(quote), self.true_quote_precision)
    }

    pub fn round_base(&self, base: ValueType) -> ValueType {
        round_float_to_precision(base, self.true_base_precision)
    }

    pub fn round_normalized_base(&self, base: ValueType) -> ValueType {
        round_float_to_precision(self.normalize_base(base), self.true_base_precision)
    }
}
