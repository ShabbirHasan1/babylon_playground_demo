use crate::{app::App, types::OrderedValueType};
use egui::Ui;
use parking_lot::RwLockReadGuard;
use ustr::Ustr;

impl App {
    pub fn ui_orders(&mut self, ui: &mut Ui, symbol: Ustr) {
        /*
        let coin_manager = self.coin_manager.read();
        let coins = coin_manager.coins();

        let coin = match coins.get(&symbol) {
            None => panic!("{}: Coin not found", symbol),
            Some(coin) => RwLockReadGuard::map(coin.read(), |m| m),
        };

        let quote_precision = coin.appraiser().true_quote_precision;
        let base_precision = coin.appraiser().true_base_precision;
         */

        /*
        let orders = self.orders.read();

        egui::ScrollArea::new([false, true]).show(ui, |ui| {
            egui::Grid::new(format!("order_list_{}", symbol)).striped(true).show(ui, |ui| {
                ui.label("Order ID");
                ui.label("Trade ID");
                ui.label("Finalized");
                ui.label("Symbol");
                ui.label("Side");
                ui.label("Type");
                ui.label("Price");
                ui.label("Initial Quantity");
                ui.label("Filled Quantity");
                ui.end_row();

                for order in orders {
                    ui.label(order.id.to_string());
                    ui.label(order.trade_id.to_string());
                    ui.label(if order.is_finalized { "Yes" } else { "No" });
                    ui.label(order.symbol.as_str());
                    ui.label(order.side.to_string()); // Ensure there is a to_string method for your enums
                    ui.label(order.order_type.to_string()); // Ensure there is a to_string method for your enums
                    ui.label(format!("{:.2}", order.price.unwrap_or(OrderedValueType(0.0)))); // Format as per your requirement
                    ui.label(format!("{:.2}", order.initial_quantity.0)); // Format as per your requirement
                    ui.label(format!("{:.2}", order.filled_quantity.0)); // Format as per your requirement
                    ui.end_row();
                }
            });
        });
         */
    }
}
