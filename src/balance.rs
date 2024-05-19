use crate::types::{Amount, OrderedValueType, Quantity};
use dashmap::{mapref::entry::Entry, DashMap};
use num_traits::Zero;
use ordered_float::OrderedFloat;
use parking_lot::{MappedMutexGuard, MappedRwLockWriteGuard, Mutex, MutexGuard, RawRwLock, RwLock, RwLockWriteGuard};
use std::sync::Arc;
use ustr::{Ustr, UstrMap};
use yata::core::ValueType;

use crate::model::Balance;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum BalanceError {
    #[error("Asset balance not found: {0}")]
    AssetNotFound(Ustr),

    #[error("Insufficient funds: requested={0:?}, available={1:?}")]
    InsufficientFunds(OrderedValueType, OrderedValueType),
}

#[derive(Clone, Debug, Default)]
pub struct BalanceSheet(Arc<Mutex<UstrMap<Balance>>>);

impl BalanceSheet {
    pub fn get(&self, asset: Ustr) -> Result<Balance, BalanceError> {
        if let Some(balance) = self.0.lock().get(&asset) {
            Ok(*balance)
        } else {
            Err(BalanceError::AssetNotFound(asset))
        }
    }

    pub fn get_mut(&self, symbol: Ustr) -> Result<MappedMutexGuard<'_, Balance>, BalanceError> {
        MutexGuard::try_map(self.0.lock(), |map| map.get_mut(&symbol)).map_err(|_| BalanceError::AssetNotFound(symbol))
    }

    pub fn set_free(&mut self, asset: Ustr, amount: OrderedValueType) {
        if let Some(balance) = self.0.lock().get_mut(&asset) {
            balance.free = amount;
            return;
        }

        self.0.lock().insert(
            asset,
            Balance {
                asset,
                free: amount,
                ..Default::default()
            },
        );
    }

    pub fn set_locked(&mut self, asset: Ustr, amount: OrderedValueType) {
        if let Some(balance) = self.0.lock().get_mut(&asset) {
            balance.locked = amount;
            return;
        }

        self.0.lock().insert(
            asset,
            Balance {
                asset,
                locked: amount,
                ..Default::default()
            },
        );
    }

    pub fn total(&self, asset: Ustr) -> Result<OrderedValueType, BalanceError> {
        let balance = self.get(asset)?;
        Ok(balance.free + balance.locked)
    }

    pub fn free(&self, asset: Ustr) -> Result<OrderedValueType, BalanceError> {
        Ok(self.get(asset)?.free)
    }

    /// Checks whether there is enough funds to perform an operation
    pub fn check(&self, asset: Ustr, amount: OrderedValueType) -> Result<bool, BalanceError> {
        Ok(self.free(asset)? >= amount)
    }

    pub fn query(&self, asset: Ustr, amount: Amount, check_funds: bool) -> Result<OrderedValueType, BalanceError> {
        let balance = self.get(asset)?;

        let value = match amount {
            Amount::Zero => OrderedFloat::zero(),
            Amount::Exact(value) => value,
            Amount::RatioOfAvailable(pc) => balance.free * pc, // Percentage value is in the range [0.0, 1.0]
            Amount::RatioOfTotal(pc) => (balance.free + balance.locked) * pc, // Percentage value is in the range [0.0, 1.0]
            Amount::AllAvailable => balance.free,
        };

        if balance.free < value {
            Err(BalanceError::InsufficientFunds(value, balance.free))
        } else {
            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_sheet_operations() {
        let mut balance_sheet = BalanceSheet::default();
        let asset = Ustr::from("USDT");

        // Initial balance check
        assert_eq!(balance_sheet.get(asset), Err(BalanceError::AssetNotFound(asset)));

        // Set free balance
        balance_sheet.set_free(asset, OrderedFloat(1000.0));
        let balance = balance_sheet.get(asset).unwrap();
        assert_eq!(balance.asset, asset);
        assert_eq!(balance.free, OrderedFloat(1000.0));
        assert_eq!(balance.locked, OrderedFloat(0.0));
        assert_eq!(balance.free + balance.locked, OrderedFloat(1000.0));

        // Set locked balance
        balance_sheet.set_locked(asset, OrderedFloat(200.0));
        let balance = balance_sheet.get(asset).unwrap();
        assert_eq!(balance.asset, asset);
        assert_eq!(balance.free, OrderedFloat(1000.0));
        assert_eq!(balance.locked, OrderedFloat(200.0));
        assert_eq!(balance.free + balance.locked, OrderedFloat(1200.0));

        // Check sufficient funds
        assert!(balance_sheet.check(asset, OrderedFloat(500.0)).unwrap());
        assert!(!balance_sheet.check(asset, OrderedFloat(1500.0)).unwrap());
    }
}
