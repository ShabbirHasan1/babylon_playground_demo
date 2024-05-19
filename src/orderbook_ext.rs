/*
use crate::orderbook::OrderBook;
use yata::core::ValueType;

#[derive(Debug)]
pub struct BidCoverage {
    // sequence index
    pub seq_idx: usize,
    // price slippage from the previous position
    pub slippage: ValueType,
    // price of the position
    pub price: ValueType,
    // gain from this position
    pub gain: ValueType,
    // initial quantity local to position
    pub initial_quantity: ValueType,
    // quantity taken from this position
    pub taken_quantity: ValueType,
    // quantity remainder in the position after taking
    pub quantity_remainder: ValueType,
}

#[derive(Debug)]
pub struct AskCoverage {
    // sequence index
    pub seq_idx: usize,
    // price slippage from the previous position
    pub slippage: ValueType,
    // price of the position
    pub price: ValueType,
    // allowance spent on this position
    pub expense: ValueType,
    // available currency
    pub available_currency: ValueType,
    // purchased amount of currency
    pub purchased_currency: ValueType,
    // initial allowance when switching to this position
    pub initial_allowance: ValueType,
    // how much allowance is left after covering this position
    pub allowance_remainder: ValueType,
    // how much currency left in this position after purchase
    pub available_currency_remainder: ValueType,
}

#[derive(Debug)]
pub struct ProjectedMarketSaleOutcome {
    // quantity to sell
    pub quantity: ValueType,
    // the quantity that left untraded
    pub quantity_remainder: ValueType,
    // total gain
    pub gain: ValueType,
    // total price slippage
    pub total_slippage: ValueType,
    // positions that were filled to cover the trade
    pub covered_bids: Vec<BidCoverage>,
}

#[derive(Debug)]
pub struct ProjectedMarketPurchaseOutcome {
    // total allowance for purchasing
    pub allowance: ValueType,
    // allowance left unspent
    pub allowance_remainder: ValueType,
    // total gain
    pub expense: ValueType,
    // purchased amount of currency
    pub purchased_amount: ValueType,
    // total price slippage
    pub total_slippage: ValueType,
    // positions that were filled to cover the trade
    pub covered_asks: Vec<AskCoverage>,
}

impl OrderBook {
    pub fn projected_market_sell_gain(&self, sell_quantity: ValueType) -> ProjectedMarketSaleOutcome {
        let mut covered_bids: Vec<BidCoverage> = vec![];
        let mut total_gain = 0.0;
        let mut seq_idx: usize = 0;
        let mut quantity_left_to_sell = sell_quantity.clone();
        let mut slippage = 0.0;
        let mut total_slippage = 0.0;
        let mut prev_price = 0.0;

        // iterating down from the best position
        for (bid_price, bid_quantity) in self.bids.iter().rev() {
            let bid_price = bid_price.into_inner();
            let bid_quantity = bid_quantity.into_inner();

            // calculating slippage from the previous position
            if seq_idx > 0 {
                slippage = prev_price - bid_price;
                total_slippage += slippage;
            }

            // is bid's quantity enough to cover the trade?
            if quantity_left_to_sell.le(&bid_quantity) {
                let bid_gain = bid_price * quantity_left_to_sell;
                total_gain += bid_gain;

                covered_bids.push(BidCoverage {
                    seq_idx,
                    price: bid_price,
                    slippage,
                    initial_quantity: bid_quantity,
                    gain: bid_gain,
                    taken_quantity: quantity_left_to_sell,
                    quantity_remainder: bid_quantity - quantity_left_to_sell,
                });

                // traded in full, zeroing
                quantity_left_to_sell = 0.0;

                break;
            } else {
                let bid_gain = bid_price * bid_quantity;
                quantity_left_to_sell -= bid_quantity;
                total_gain += bid_gain;

                covered_bids.push(BidCoverage {
                    seq_idx,
                    price: bid_price,
                    slippage,
                    initial_quantity: bid_quantity,
                    gain: bid_gain,
                    taken_quantity: bid_quantity,
                    quantity_remainder: bid_quantity - quantity_left_to_sell,
                });
            }

            prev_price = bid_price;
            seq_idx = seq_idx.saturating_add(1);
        }

        ProjectedMarketSaleOutcome {
            quantity: sell_quantity,
            quantity_remainder: quantity_left_to_sell,
            gain: total_gain,
            total_slippage,
            covered_bids,
        }
    }

    pub fn projected_market_buy_expense(
        &self,
        allowance: ValueType,
    ) -> ProjectedMarketPurchaseOutcome {
        let mut covered_asks: Vec<AskCoverage> = vec![];
        let mut total_expense: ValueType = 0.0;
        let mut total_currency: ValueType = 0.0;
        let mut seq_idx: usize = 0;
        let mut allowance_left_unspent = allowance;
        let mut slippage = 0.0;
        let mut total_slippage = 0.0;
        let mut prev_price = 0.0;

        // iterating up from the best position
        for (ask_price, ask_quantity) in self.asks.iter() {
            let ask_price = ask_price.into_inner();
            let ask_quantity = ask_quantity.into_inner();

            // calculating slippage from the previous position
            if seq_idx > 0 {
                slippage = prev_price - ask_price.abs();
                total_slippage += slippage;
            }

            let total_ask_expense = ask_quantity * ask_price;

            // does this ask covers the allowance for purchase
            if allowance_left_unspent.le(&total_ask_expense) {
                let purchased_currency = allowance_left_unspent / ask_price;
                let ask_expense = purchased_currency * ask_price;
                total_expense += ask_expense;
                total_currency += purchased_currency;

                covered_asks.push(AskCoverage {
                    seq_idx,
                    slippage,
                    price: ask_price,
                    expense: ask_expense,
                    available_currency: ask_quantity,
                    purchased_currency,
                    initial_allowance: allowance_left_unspent,
                    allowance_remainder: 0.0,
                    available_currency_remainder: ask_quantity - purchased_currency,
                });

                allowance_left_unspent -= ask_expense;

                break;
            } else {
                let purchased_currency = ask_quantity;
                let ask_expense = purchased_currency * ask_price;
                total_expense += ask_expense;
                total_currency += purchased_currency;

                covered_asks.push(AskCoverage {
                    seq_idx,
                    slippage,
                    price: ask_price,
                    expense: ask_expense,
                    available_currency: ask_quantity,
                    purchased_currency,
                    initial_allowance: allowance_left_unspent,
                    allowance_remainder: allowance_left_unspent - ask_expense,
                    available_currency_remainder: ask_quantity - purchased_currency,
                });

                allowance_left_unspent -= ask_expense;
            }

            prev_price = ask_price;
            seq_idx = seq_idx.saturating_add(1);
        }

        ProjectedMarketPurchaseOutcome {
            allowance,
            allowance_remainder: allowance_left_unspent,
            expense: total_expense,
            purchased_amount: total_currency,
            total_slippage,
            covered_asks,
        }
    }
}
 */
