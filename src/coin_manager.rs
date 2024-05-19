use std::{borrow::Borrow, collections::hash_map::Entry, sync::Arc};

use crate::{coin::Coin, context::ContextError};
use anyhow::Result;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard, RwLock};
use ustr::{Ustr, UstrMap};

#[derive(Clone, Default)]
pub struct CoinManager(Arc<Mutex<UstrMap<Arc<RwLock<Coin>>>>>);

impl CoinManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_coin(&mut self, symbol: Ustr, coin: Arc<RwLock<Coin>>) -> Result<(), ContextError> {
        if let Entry::Vacant(mut e) = self.0.lock().entry(symbol) {
            e.insert(coin);
        }

        Ok(())
    }

    pub fn coins(&self) -> MappedMutexGuard<'_, UstrMap<Arc<RwLock<Coin>>>> {
        MutexGuard::map(self.0.lock(), |m| m)
    }
}
