use crate::types::OrderedValueType;

#[derive(Debug, Copy, Clone)]
pub struct LeverageData {
    /// Details about leverage ratio, borrow fee, and borrow price
    ratio:        OrderedValueType,
    borrow_fee:   OrderedValueType,
    borrow_price: OrderedValueType,
}

#[derive(Debug, Copy, Clone)]
pub struct Leverage {
    /// Leverage data. None if leverage isn't used.
    leverage_data: Option<LeverageData>,

    /// Confirms that the borrowed quantity has been repaid.
    repaid: bool,
}

impl Leverage {
    /// Use this for explicitly for the peace of mind.
    pub fn no_leverage() -> Self {
        Self {
            leverage_data: None,
            repaid:        false,
        }
    }

    /// Initialize config with leverage ENABLED.
    /// WARNING: think 3 times
    pub fn with_leverage(ratio: OrderedValueType, borrow_fee: OrderedValueType, borrow_price: OrderedValueType) -> Self {
        assert!(ratio.0 > 0.0 && ratio.0 <= 100.0, "Invalid leverage ratio"); // Check if ratio is within a sensible range
        assert!(borrow_fee.0 >= 0.0 && borrow_fee.0 < 1.0, "Invalid borrow fee"); // Check if borrow_fee is within a sensible range

        Self {
            leverage_data: Some(LeverageData {
                ratio,
                borrow_fee,
                borrow_price,
            }),
            repaid:        false,
        }
    }

    /// Confirm that the borrowed quantity has been repaid.
    pub fn confirm_repayment(&mut self) {
        assert!(self.leverage_data.is_some(), "Repayment was not expected for this trade");
        self.repaid = true;
    }

    /// Get the leverage ratio.
    pub fn ratio(&self) -> Option<OrderedValueType> {
        self.leverage_data.map(|data| data.ratio)
    }

    /// Get the borrow fee.
    pub fn borrow_fee(&self) -> Option<OrderedValueType> {
        self.leverage_data.map(|data| data.borrow_fee)
    }

    /// Get the borrow price.
    pub fn borrow_price(&self) -> Option<OrderedValueType> {
        self.leverage_data.map(|data| data.borrow_price)
    }

    /// Check if the borrowed quantity has been repaid.
    pub fn is_repaid(&self) -> bool {
        self.repaid
    }

    /// Check if leverage is used.
    pub fn is_leveraged(&self) -> bool {
        self.leverage_data.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ordered_float::OrderedFloat;

    #[test]
    fn no_leverage_test() {
        let leverage = Leverage::no_leverage();
        assert!(leverage.leverage_data.is_none());
        assert!(leverage.repaid);
    }

    #[test]
    fn with_leverage_test() {
        let ratio = OrderedFloat(10.0);
        let borrow_fee = OrderedFloat(0.01);
        let borrow_price = OrderedFloat(35_000.0);

        let leverage = Leverage::with_leverage(ratio, borrow_fee, borrow_price);

        assert!(leverage.leverage_data.is_some());
        let leverage_data = leverage.leverage_data.unwrap();
        assert_eq!(leverage_data.ratio, ratio);
        assert_eq!(leverage_data.borrow_fee, borrow_fee);
        assert_eq!(leverage_data.borrow_price, borrow_price);
        assert!(!leverage.repaid);
    }
}
