use std::{
    cmp::Ordering,
    fmt::{Debug, Display, Formatter},
    time::{Duration, Instant},
};

use crate::types::OrderedValueType;
use log::info;
use num_traits::Zero;
use ordered_float::OrderedFloat;
use yata::core::{PeriodType, ValueType, Window};

pub const LEVEL_LIFETIME: Duration = Duration::new(3, 0);

#[derive(Debug, Copy, Clone)]
#[repr(u8)]
pub enum LevelStrength {
    Irrelevant = 0,
    Weak = 1,
    Moderate = 2,
    Strong = 3,
    Bedrock = 4,
}

#[derive(Debug, Copy, Clone, Ord, Eq)]
pub struct OrderBookLevel {
    pub price:    OrderedValueType,
    pub quantity: OrderedValueType,
    // pub strength: LevelStrength,
}

impl Default for OrderBookLevel {
    fn default() -> Self {
        OrderBookLevel::new(OrderedFloat::zero(), OrderedFloat::zero())
    }
}

impl PartialEq<Self> for OrderBookLevel {
    fn eq(&self, other: &Self) -> bool {
        self.price.eq(&other.price)
    }
}

impl PartialOrd for OrderBookLevel {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.price.cmp(&other.price))
    }
}

impl OrderBookLevel {
    pub(crate) const WINDOW_SIZE: PeriodType = 10;

    pub fn new(price: OrderedValueType, quantity: OrderedValueType) -> Self {
        Self {
            price,
            quantity,
            // strength: LevelStrength::Irrelevant,
        }
    }

    pub fn update(&mut self, new_quantity: OrderedValueType) {
        self.quantity = new_quantity;
    }
}
