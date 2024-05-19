use ordered_float::OrderedFloat;
use yata::core::ValueType;

#[derive(Debug, Clone)]
pub struct LevelDelta {
    pub price:        ValueType,
    pub new_quantity: ValueType,
    pub delta:        ValueType,
    pub is_expended:  bool,
}

#[derive(Debug, Clone, Default)]
pub struct OrderBookDelta {
    pub asks: Vec<LevelDelta>,
    pub bids: Vec<LevelDelta>,
}
