use crate::{
    balance::BalanceSheet, d, id::OrderId, instruction::ExecutionOutcome, model::Balance, position_accumulator::AccumulatorMetadata, types::NormalizedSide,
};
use num_traits::FromPrimitive;
use pausable_clock::PausableClock;
use rust_decimal::Decimal;
use std::{collections::HashSet, time::Duration};
use thiserror::Error;
use ustr::Ustr;

#[derive(Debug, Error, PartialEq)]
pub enum PositionError {
    #[error("Order ID already exists in pending order confirmations set")]
    OrderIdExists,

    #[error("Insufficient amount held to sell")]
    InsufficientAmount,

    #[error("Insufficient capital to buy")]
    InsufficientCapital,

    #[error("Insufficient capital to reserve")]
    InsufficientCapitalToReserve,

    #[error("Order ID not found in pending order confirmations set")]
    OrderIdNotFound,
}

#[derive(Debug, PartialEq, Default)]
pub enum TradeDecisionAmount {
    #[default]
    None,
    Ratio(f64),
    Exact(f64),
    Tenth,
    Fifth,
    Quarter,
    Third,
    Half,
    TwoThirds,
    Full,
}

#[derive(Debug, PartialEq, Default)]
pub enum TradeDecision {
    #[default]
    Hold,
    Buy(TradeDecisionAmount),
    Sell(TradeDecisionAmount),
}

#[derive(Debug)]
pub struct Position {
    /// Symbol of the asset being traded.
    symbol: Ustr,

    /// Needed to have immediate access to the balance for the position and to be able to reserve funds for pending orders.
    balance: BalanceSheet,

    /// Capital that has been allocated to this position.
    allocated_capital: Decimal,

    /// Capital that has been used to purchase assets.
    invested_capital: Decimal,

    /// Capital that has been reserved for pending orders.
    reserved_capital: Decimal,

    /// Revenue from sold assets.
    revenue: Decimal,

    /// Ratio of the invested capital that can be risked.
    risk_tolerance_ratio: Decimal,

    /// Amount of the asset held.
    quantity_held: Decimal,

    /// Average price of the asset.
    average_price: Decimal,

    /// Realized profit.
    realized_profit: Decimal,

    /// Realized profit as a ratio of the invested capital.
    realized_profit_ratio: Decimal,

    /// Unrealized profit based on the current market price.
    unrealized_profit: Decimal,

    /// Unrealized profit as a ratio of the invested capital.
    unrealized_profit_ratio: Decimal,

    /// Set of pending order IDs that are awaiting confirmation.
    pending_order_confirmations: HashSet<OrderId>,

    /// A pausable clock to manage the position's state (e.g. delayed stop-loss).
    clock: PausableClock,

    /// Accumulator metadata for the position.
    /// TODO: Decide if this is needed.
    accumulator: AccumulatorMetadata,
}

impl Position {
    /// Constructs a new trading position.
    pub fn new(symbol: Ustr, balance: BalanceSheet) -> Self {
        Self {
            symbol,
            balance,
            allocated_capital: Decimal::ZERO,
            invested_capital: Decimal::ZERO,
            reserved_capital: Decimal::ZERO,
            revenue: Decimal::ZERO,
            risk_tolerance_ratio: Decimal::ZERO,
            quantity_held: Decimal::ZERO,
            average_price: Decimal::ZERO,
            realized_profit: Decimal::ZERO,
            realized_profit_ratio: Decimal::ZERO,
            unrealized_profit: Decimal::ZERO,
            unrealized_profit_ratio: Decimal::ZERO,
            pending_order_confirmations: Default::default(),
            clock: PausableClock::new(Duration::new(15, 0), true),
            accumulator: Default::default(),
        }
    }

    pub fn track_pending_order(&mut self, order_id: OrderId) -> Result<(), PositionError> {
        if self.pending_order_confirmations.contains(&order_id) {
            return Err(PositionError::OrderIdExists);
        }

        self.pending_order_confirmations.insert(order_id);

        Ok(())
    }

    pub fn notify_order_finished(&mut self, order_id: OrderId) -> Result<(), PositionError> {
        if !self.pending_order_confirmations.contains(&order_id) {
            return Err(PositionError::OrderIdNotFound);
        }

        self.pending_order_confirmations.remove(&order_id);

        Ok(())
    }

    /// Updates the position's state based on a completed buy transaction.
    fn update_on_buy(&mut self, amount: f64, price: f64) {
        let amount = d!(amount);
        let price = d!(price);
        let total_cost_post_transaction = self.average_price * self.quantity_held + amount * price;

        self.invested_capital += amount * price;
        self.allocated_capital += amount * price;
        self.quantity_held += amount;
        self.average_price = if !self.quantity_held.is_zero() { total_cost_post_transaction / self.quantity_held } else { Decimal::ZERO };

        println!("B: +{} at {}, new average cost: {}, total held: {}", amount, price, self.average_price, self.quantity_held);
    }

    /// Updates the position's state based on a completed sell transaction.
    fn update_on_sell(&mut self, amount: f64, price: f64) {
        let amount = d!(amount);
        let price = d!(price);
        let revenue = amount * price;
        let profit = (price - self.average_price) * amount;

        self.revenue += revenue;
        self.realized_profit += profit;
        self.quantity_held -= amount;

        let (roi, roi_ratio) = self.calculate_roi();

        println!("S: -{} at {}, realized profit: {} ({:.2}%), total held: {}", amount, price, profit, roi_ratio * d!(100.0), self.quantity_held);
    }

    pub fn wait_for_pending_order_confirmation(&mut self, order_id: OrderId) -> Result<(), PositionError> {
        if !self.pending_order_confirmations.contains(&order_id) {
            return Err(PositionError::InsufficientAmount);
        }

        self.pending_order_confirmations.insert(order_id);

        Ok(())
    }

    /// Updates the position's state based on a completed trade transaction.
    #[inline]
    pub fn register_trade(&mut self, side: NormalizedSide, amount: f64, price: f64) {
        match side {
            NormalizedSide::Buy => self.update_on_buy(amount, price),
            NormalizedSide::Sell => self.update_on_sell(amount, price),
        }
    }

    /// Calculate the current unrealized profit based on the current market price.
    pub fn calculate_unrealized_profit(&self, current_price: Decimal) -> (Decimal, Decimal) {
        if self.quantity_held.is_zero() {
            (Decimal::ZERO, Decimal::ZERO)
        } else {
            let unrealized_profit = (current_price - self.average_price) * self.quantity_held;
            let unrealized_profit_ratio = (unrealized_profit / self.invested_capital) * d!(100.0);
            (unrealized_profit, unrealized_profit_ratio)
        }
    }

    /// Calculate return on investment (ROI) based on the initial capital.
    pub fn calculate_roi(&self) -> (Decimal, Decimal) {
        if self.invested_capital.is_zero() {
            (Decimal::ZERO, Decimal::ZERO)
        } else {
            let roi = ((self.revenue + self.calculate_unrealized_profit(self.average_price).0 - self.invested_capital) / self.invested_capital) * d!(100.0);
            let roi_ratio = (self.revenue + self.calculate_unrealized_profit(self.average_price).0 - self.invested_capital) / self.invested_capital;
            (roi, roi_ratio)
        }
    }

    /// Calculates the recommended size for the next trade based on the current market volatility and risk tolerance.
    pub fn calculate_trade_size(&self, volatility_ratio: f64) -> Decimal {
        let volatility = d!(volatility_ratio);
        let risk_amount = self.allocated_capital * self.risk_tolerance_ratio;
        let position_size = risk_amount / volatility; // Higher volatility -> smaller position size
        position_size
    }

    /// Determines if a buy order can be placed within the current allocation.
    pub fn can_buy_within_allocation(&self, quantity: f64, price: f64) -> bool {
        let purchase_cost = d!(quantity) * d!(price);
        let remaining_capital = self.allocated_capital - self.invested_capital;
        purchase_cost <= remaining_capital
    }

    /// Computes the current state of the position (e.g., unrealized profit) based on the current market price.
    pub fn compute(&mut self, current_price: f64) -> Result<TradeDecision, PositionError> {
        let current_price = d!(current_price);
        self.unrealized_profit = (current_price - self.average_price) * self.quantity_held;

        // TODO: Add logic to determine trade decision based on unrealized profit

        Ok(TradeDecision::Hold)
    }
}

#[cfg(test)]
mod tests {
    use crate::position::{Position, *};
    use ustr::ustr; // Ensures precise decimal operations in tests

    #[test]
    fn test_buy_update() {
        let mut position = Position::new(ustr("TESTUSDT"));
        position.update_on_buy(1.0, 100.0);
        assert_eq!(position.quantity_held, d!(1.0));
        assert_eq!(position.average_price, d!(100.0));
    }

    #[test]
    fn test_sell_update() {
        let mut position = Position::new(ustr("TESTUSDT"));
        position.update_on_buy(2.0, 100.0); // Buy 2 BTC at $100 each
        position.update_on_sell(1.0, 120.0); // Sell 1 BTC at $120

        assert_eq!(position.quantity_held, d!(1.0));
        assert_eq!(position.realized_profit, d!(20.0));
    }

    #[test]
    fn test_unrealized_profit_update() {
        let mut position = Position::new(ustr("TESTUSDT"));
        position.update_on_buy(1.0, 100.0); // Buy 1 BTC at $100
        position.compute(150.0); // Current market price is $150

        assert_eq!(position.unrealized_profit, d!(50.0));
    }

    // Test multiple buys at different prices and a single sell
    #[test]
    fn multiple_buys_and_one_sell() {
        let mut position = Position::new(ustr("TESTUSDT"));

        // First buy
        position.update_on_buy(1.0, 100.0);
        // Second buy, higher price, changes average cost
        position.update_on_buy(1.0, 120.0);

        // Average cost should now be (1*100 + 1*120) / (1 + 1) = 110
        assert_eq!(position.average_price, d!(110.0));
        assert_eq!(position.quantity_held, d!(2.0));

        // Sell half at a higher price
        position.update_on_sell(1.0, 130.0);

        // Realized profit should be (130 - 110) * 1 = 20
        assert_eq!(position.realized_profit, d!(20.0));
        assert_eq!(position.quantity_held, d!(1.0));
        // Average cost remains the same after sale
        assert_eq!(position.average_price, d!(110.0));
    }

    // Test updating unrealized profit with fluctuating market prices
    #[test]
    fn unrealized_profit_with_fluctuating_prices() {
        let mut position = Position::new(ustr("TESTUSDT"));
        position.update_on_buy(2.0, 100.0);

        // Increase in price
        position.compute(110.0);
        assert_eq!(position.unrealized_profit, d!(20.0)); // (110 - 100) * 2

        // Price drops below average cost
        position.compute(90.0);
        assert_eq!(position.unrealized_profit, d!(-20.0)); // (90 - 100) * 2
    }

    // Test the effect of a buy transaction on unrealized profit
    #[test]
    fn buy_impact_on_unrealized_profit() {
        let mut position = Position::new(ustr("TESTUSDT"));
        position.update_on_buy(1.0, 100.0);
        position.compute(150.0); // Unrealized profit is 50

        // Buying more at a higher price reduces unrealized profit
        position.update_on_buy(1.0, 200.0); // Average cost now (100 + 200) / 2 = 150
        position.compute(150.0); // New price matches new average cost

        assert_eq!(position.unrealized_profit, d!(0.0)); // Price equals average cost, so unrealized profit is zero
    }

    // Test handling multiple buy fills
    #[test]
    fn multiple_buy_fills() {
        let mut position = Position::new(ustr("TESTUSDT"));

        // Simulating multiple fills from a market buy order
        position.update_on_buy(1.0, 100.0); // First fill
        position.update_on_buy(2.0, 105.0); // Second fill with a different price

        assert_eq!(position.quantity_held, d!(3.0));
        // Expected average cost: (1*100 + 2*105) / 3 = 103.33...
        assert_eq!(position.average_price.round_dp(2), d!(103.33));
    }

    // Test handling a sell that spans multiple fills
    #[test]
    fn multiple_sell_fills() {
        let mut position = Position::new(ustr("TESTUSDT"));
        // Starting with an existing position
        position.update_on_buy(3.0, 100.0);

        // Simulating multiple fills from a market sell order
        position.update_on_sell(1.0, 110.0); // First fill
        position.update_on_sell(1.0, 115.0); // Second fill with a different price

        assert_eq!(position.quantity_held, d!(1.0)); // 3 - 1 - 1 = 1 remaining
                                                     // Expected realized profit: (110-100)*1 + (115-100)*1 = 25
        assert_eq!(position.realized_profit, d!(25.0));
    }

    // Test updating unrealized profit after multiple operations
    #[test]
    fn unrealized_profit_after_operations() {
        let mut position = Position::new(ustr("TESTUSDT"));
        position.update_on_buy(2.0, 100.0);
        position.update_on_buy(1.0, 110.0); // Adjusting average cost
        position.update_on_sell(1.0, 120.0); // Realizing some profit

        // Market price increases further
        position.compute(130.0);

        // Corrected expected unrealized profit calculation after the sell
        assert_eq!(position.unrealized_profit.round_dp(2), d!(53.33)); // Rounded to 2 decimal places for assertion
    }

    #[test]
    fn test_sell_updates_position_correctly() {
        let mut position = Position::new(ustr("BTC"));
        position.update_on_buy(2.0, 100.0);
        position.update_on_sell(1.0, 120.0);

        assert_eq!(position.quantity_held, d!(1.0), "Amount held should be updated correctly");
        assert_eq!(position.realized_profit, d!(20.0), "Realized profit should be 20.0");
        assert_eq!(position.revenue, d!(120.0), "Total revenue should be 120.0 after selling one unit at 120");
    }

    #[test]
    fn test_unrealized_profit_calculation() {
        let mut position = Position::new(ustr("BTC"));
        position.update_on_buy(1.0, 100.0);
        position.update_on_buy(1.0, 200.0); // Buy another at higher price, average cost now 150

        let current_price = d!(250.0);
        let (unrealized_profit, unrealized_profit_ratio) = position.calculate_unrealized_profit(current_price);
        assert_eq!(unrealized_profit, d!(200.0)); // (250-150)*2
    }

    #[test]
    fn test_roi_calculation() {
        let mut position = Position::new(ustr("BTC"));
        position.allocated_capital = Decimal::from(1000); // Set initial capital for clarity, if used in ROI calculation.
        position.update_on_buy(10.0, 10.0); // Buying 10 units at $10 each, total cost $100
        position.update_on_sell(10.0, 15.0); // Selling 10 units at $15 each, total revenue $150

        // Ensure invested capital and total revenue are updated correctly
        assert_eq!(position.invested_capital, Decimal::from(100)); // $100 used for buying
        assert_eq!(position.revenue, Decimal::from(150)); // $150 gained from selling

        // Calculate ROI
        let (roi, roi_ratio) = position.calculate_roi();
        assert_eq!(roi, Decimal::from(50), "ROI should be 50%"); // Expecting a 50% ROI
    }

    #[test]
    fn test_unrealized_profit_updates_with_market_price() {
        let mut position = Position::new(ustr("BTC"));
        position.update_on_buy(1.0, 100.0);

        // Market price increase
        position.compute(150.0);
        assert_eq!(position.unrealized_profit, d!(50.0)); // Unrealized profit update

        // Market price decrease
        position.compute(90.0);
        assert_eq!(position.unrealized_profit, d!(-10.0)); // Unrealized loss
    }

    #[test]
    fn multiple_buys_and_sells_affect_financials_correctly() {
        let mut position = Position::new(ustr("BTC"));
        position.update_on_buy(1.0, 100.0);
        position.update_on_buy(1.0, 200.0); // Average cost now 150
        position.update_on_sell(1.0, 300.0); // Selling one unit

        assert_eq!(position.quantity_held, d!(1.0)); // 2 bought, 1 sold
        assert_eq!(position.realized_profit, d!(150.0)); // Profit from the sale
        assert_eq!(position.revenue, d!(300.0)); // Revenue from the sale
    }
}
