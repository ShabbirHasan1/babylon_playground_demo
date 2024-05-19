use crate::timeframe::{Timeframe, TimeframeError};
use chrono::format::Item;
use log::error;
use nohash_hasher::IntMap;
use serde_derive::{Deserialize, Serialize};
use std::{borrow::Borrow, collections::hash_map::Entry, ops::Index};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TimeSet<T: Clone + Send + Sync> {
    pub s1:  Option<T>,
    pub m1:  Option<T>,
    pub m3:  Option<T>,
    pub m5:  Option<T>,
    pub m15: Option<T>,
    pub m30: Option<T>,
    pub h1:  Option<T>,
    pub h2:  Option<T>,
    pub h4:  Option<T>,
    pub h6:  Option<T>,
    pub h8:  Option<T>,
    pub h12: Option<T>,
    pub d1:  Option<T>,
    pub d3:  Option<T>,
    pub w1:  Option<T>,
    pub mm1: Option<T>,
}

impl<T: Clone + Send + Sync> Index<u8> for TimeSet<T> {
    type Output = Option<T>;

    fn index(&self, index: u8) -> &Self::Output {
        match index {
            0 => &self.s1,
            1 => &self.m1,
            2 => &self.m3,
            3 => &self.m5,
            4 => &self.m15,
            5 => &self.m30,
            6 => &self.h1,
            7 => &self.h2,
            8 => &self.h4,
            9 => &self.h6,
            10 => &self.h8,
            11 => &self.h12,
            12 => &self.d1,
            13 => &self.d3,
            14 => &self.w1,
            15 => &self.mm1,
            _ => {
                error!("invalid timeset index: {}", index);
                &None
            }
        }
    }
}

impl<T: Clone + Send + Sync> Index<Timeframe> for TimeSet<T> {
    type Output = Option<T>;

    fn index(&self, timeframe: Timeframe) -> &Self::Output {
        match timeframe {
            Timeframe::S1 => &self.s1,
            Timeframe::M1 => &self.m1,
            Timeframe::M3 => &self.m3,
            Timeframe::M5 => &self.m5,
            Timeframe::M15 => &self.m15,
            Timeframe::M30 => &self.m30,
            Timeframe::H1 => &self.h1,
            Timeframe::H2 => &self.h2,
            Timeframe::H4 => &self.h4,
            Timeframe::H6 => &self.h6,
            Timeframe::H8 => &self.h8,
            Timeframe::H12 => &self.h12,
            Timeframe::D1 => &self.d1,
            Timeframe::D3 => &self.d3,
            Timeframe::W1 => &self.mm1,
            Timeframe::MM1 => &self.mm1,
            _ => {
                error!("invalid timeset timeframe: {:?}", timeframe);
                &None
            }
        }
    }
}

impl<T: Clone + Send + Sync> Index<&str> for TimeSet<T> {
    type Output = Option<T>;

    fn index(&self, timeframe: &str) -> &Self::Output {
        match timeframe {
            "1s" => &self.s1,
            "1m" => &self.m1,
            "3m" => &self.m3,
            "5m" => &self.m5,
            "15m" => &self.m15,
            "30m" => &self.m30,
            "1h" => &self.h1,
            "2h" => &self.h2,
            "4h" => &self.h4,
            "6h" => &self.h6,
            "8h" => &self.h8,
            "12h" => &self.h12,
            "1d" => &self.d1,
            "3d" => &self.d3,
            "1w" => &self.mm1,
            "1M" => &self.mm1,
            _ => {
                error!("invalid timeset timeframe: {:?}", timeframe);
                &None
            }
        }
    }
}

impl<T: Clone + Send + Sync> TimeSet<T> {
    pub fn new() -> Self {
        Self {
            s1:  None,
            m1:  None,
            m3:  None,
            m5:  None,
            m15: None,
            m30: None,
            h1:  None,
            h2:  None,
            h4:  None,
            h6:  None,
            h8:  None,
            h12: None,
            d1:  None,
            d3:  None,
            w1:  None,
            mm1: None,
        }
    }

    pub fn new_with(value: T) -> Self {
        Self {
            s1:  Some(value.clone()),
            m1:  Some(value.clone()),
            m3:  Some(value.clone()),
            m5:  Some(value.clone()),
            m15: Some(value.clone()),
            m30: Some(value.clone()),
            h1:  Some(value.clone()),
            h2:  Some(value.clone()),
            h4:  Some(value.clone()),
            h6:  Some(value.clone()),
            h8:  Some(value.clone()),
            h12: Some(value.clone()),
            d1:  Some(value.clone()),
            d3:  Some(value.clone()),
            w1:  Some(value.clone()),
            mm1: Some(value.clone()),
        }
    }

    pub fn supported_timeframes(&self) -> [Timeframe; 16] {
        [
            Timeframe::S1,
            Timeframe::M1,
            Timeframe::M3,
            Timeframe::M5,
            Timeframe::M15,
            Timeframe::M30,
            Timeframe::H1,
            Timeframe::H2,
            Timeframe::H4,
            Timeframe::H6,
            Timeframe::H8,
            Timeframe::H12,
            Timeframe::D1,
            Timeframe::D3,
            Timeframe::W1,
            Timeframe::MM1,
        ]
    }

    pub fn trend_timeframe(&self, timeframe: Timeframe) -> Timeframe {
        match timeframe {
            Timeframe::S1 => Timeframe::M1,   // Scalping
            Timeframe::M1 => Timeframe::M5,   // Scalping
            Timeframe::M3 => Timeframe::M15,  // Scalping
            Timeframe::M5 => Timeframe::H1,   // Short-term
            Timeframe::M15 => Timeframe::H1,  // Day trading
            Timeframe::M30 => Timeframe::H2,  // Day trading
            Timeframe::H1 => Timeframe::H4,   // Day trading
            Timeframe::H2 => Timeframe::H8,   // Short-term
            Timeframe::H4 => Timeframe::D1,   // Short-term
            Timeframe::H6 => Timeframe::D1,   // Swing trading
            Timeframe::H8 => Timeframe::D1,   // Swing trading
            Timeframe::H12 => Timeframe::D3,  // Swing trading
            Timeframe::D1 => Timeframe::W1,   // Swing trading
            Timeframe::D3 => Timeframe::W1,   // Long-term
            Timeframe::W1 => Timeframe::MM1,  // Long-term
            Timeframe::MM1 => Timeframe::MM1, // Long-term (stays the same as no larger timeframe is available)
            _ => Timeframe::NA,
        }
    }

    pub fn set(&mut self, timeframe: Timeframe, value: T) -> Result<(), TimeframeError> {
        match timeframe {
            Timeframe::S1 => self.s1 = Some(value),
            Timeframe::M1 => self.m1 = Some(value),
            Timeframe::M3 => self.m3 = Some(value),
            Timeframe::M5 => self.m5 = Some(value),
            Timeframe::M15 => self.m15 = Some(value),
            Timeframe::M30 => self.m30 = Some(value),
            Timeframe::H1 => self.h1 = Some(value),
            Timeframe::H2 => self.h2 = Some(value),
            Timeframe::H4 => self.h4 = Some(value),
            Timeframe::H6 => self.h6 = Some(value),
            Timeframe::H8 => self.h8 = Some(value),
            Timeframe::H12 => self.h12 = Some(value),
            Timeframe::D1 => self.d1 = Some(value),
            Timeframe::D3 => self.d3 = Some(value),
            Timeframe::W1 => self.w1 = Some(value),
            Timeframe::MM1 => self.mm1 = Some(value),
            _ => {
                return Err(TimeframeError::TimeframeNotFound(timeframe));
            }
        }

        Ok(())
    }

    pub fn get(&self, timeframe: Timeframe) -> Result<&T, TimeframeError> {
        match timeframe {
            Timeframe::S1 => self.s1.as_ref(),
            Timeframe::M1 => self.m1.as_ref(),
            Timeframe::M3 => self.m3.as_ref(),
            Timeframe::M5 => self.m5.as_ref(),
            Timeframe::M15 => self.m15.as_ref(),
            Timeframe::M30 => self.m30.as_ref(),
            Timeframe::H1 => self.h1.as_ref(),
            Timeframe::H2 => self.h2.as_ref(),
            Timeframe::H4 => self.h4.as_ref(),
            Timeframe::H6 => self.h6.as_ref(),
            Timeframe::H8 => self.h8.as_ref(),
            Timeframe::H12 => self.h12.as_ref(),
            Timeframe::D1 => self.d1.as_ref(),
            Timeframe::D3 => self.d3.as_ref(),
            Timeframe::W1 => self.w1.as_ref(),
            Timeframe::MM1 => self.mm1.as_ref(),
            _ => return Err(TimeframeError::TimeframeNotFound(timeframe)),
        }
        .ok_or(TimeframeError::TimeframeNotFound(timeframe))
    }

    pub fn get_mut(&mut self, timeframe: Timeframe) -> Result<&mut T, TimeframeError> {
        match timeframe {
            Timeframe::S1 => self.s1.as_mut(),
            Timeframe::M1 => self.m1.as_mut(),
            Timeframe::M3 => self.m3.as_mut(),
            Timeframe::M5 => self.m5.as_mut(),
            Timeframe::M15 => self.m15.as_mut(),
            Timeframe::M30 => self.m30.as_mut(),
            Timeframe::H1 => self.h1.as_mut(),
            Timeframe::H2 => self.h2.as_mut(),
            Timeframe::H4 => self.h4.as_mut(),
            Timeframe::H6 => self.h6.as_mut(),
            Timeframe::H8 => self.h8.as_mut(),
            Timeframe::H12 => self.h12.as_mut(),
            Timeframe::D1 => self.d1.as_mut(),
            Timeframe::D3 => self.d3.as_mut(),
            Timeframe::W1 => self.w1.as_mut(),
            Timeframe::MM1 => self.mm1.as_mut(),
            _ => return Err(TimeframeError::TimeframeNotFound(timeframe)),
        }
        .ok_or(TimeframeError::TimeframeNotFound(timeframe))
    }

    pub fn clear(&mut self, timeframe: Timeframe) -> Result<(), TimeframeError> {
        match timeframe {
            Timeframe::S1 => self.s1 = None,
            Timeframe::M1 => self.m1 = None,
            Timeframe::M3 => self.m3 = None,
            Timeframe::M5 => self.m5 = None,
            Timeframe::M15 => self.m15 = None,
            Timeframe::M30 => self.m30 = None,
            Timeframe::H1 => self.h1 = None,
            Timeframe::H2 => self.h2 = None,
            Timeframe::H4 => self.h4 = None,
            Timeframe::H6 => self.h6 = None,
            Timeframe::H8 => self.h8 = None,
            Timeframe::H12 => self.h12 = None,
            Timeframe::D1 => self.d1 = None,
            Timeframe::D3 => self.d3 = None,
            Timeframe::W1 => self.w1 = None,
            Timeframe::MM1 => self.mm1 = None,
            _ => return Err(TimeframeError::TimeframeNotFound(timeframe)),
        }

        Ok(())
    }

    pub fn map<F, U>(&self, f: F) -> TimeSet<U>
    where
        F: Fn(&T) -> U,
        U: Clone + Send + Sync,
    {
        TimeSet {
            s1:  self.s1.as_ref().map(&f),
            m1:  self.m1.as_ref().map(&f),
            m3:  self.m3.as_ref().map(&f),
            m5:  self.m5.as_ref().map(&f),
            m15: self.m15.as_ref().map(&f),
            m30: self.m30.as_ref().map(&f),
            h1:  self.h1.as_ref().map(&f),
            h2:  self.h2.as_ref().map(&f),
            h4:  self.h4.as_ref().map(&f),
            h6:  self.h6.as_ref().map(&f),
            h8:  self.h8.as_ref().map(&f),
            h12: self.h12.as_ref().map(&f),
            d1:  self.d1.as_ref().map(&f),
            d3:  self.d3.as_ref().map(&f),
            w1:  self.w1.as_ref().map(&f),
            mm1: self.mm1.as_ref().map(&f),
        }
    }

    pub fn filter<F>(&self, f: F) -> Vec<(Timeframe, &T)>
    where
        F: Fn(&T) -> bool,
    {
        let mut out = Vec::<(Timeframe, &T)>::with_capacity(16);

        if let Some(ref value) = self.s1 {
            if f(value) {
                out.push((Timeframe::S1, value));
            }
        }

        if let Some(ref value) = self.m1 {
            if f(value) {
                out.push((Timeframe::M1, value));
            }
        }

        if let Some(ref value) = self.m3 {
            if f(value) {
                out.push((Timeframe::M3, value));
            }
        }

        if let Some(ref value) = self.m5 {
            if f(value) {
                out.push((Timeframe::M5, value));
            }
        }

        if let Some(ref value) = self.m15 {
            if f(value) {
                out.push((Timeframe::M15, value));
            }
        }

        if let Some(ref value) = self.m30 {
            if f(value) {
                out.push((Timeframe::M30, value));
            }
        }

        if let Some(ref value) = self.h1 {
            if f(value) {
                out.push((Timeframe::H1, value));
            }
        }

        if let Some(ref value) = self.h2 {
            if f(value) {
                out.push((Timeframe::H2, value));
            }
        }

        if let Some(ref value) = self.h4 {
            if f(value) {
                out.push((Timeframe::H4, value));
            }
        }

        if let Some(ref value) = self.h6 {
            if f(value) {
                out.push((Timeframe::H6, value));
            }
        }

        if let Some(ref value) = self.h8 {
            if f(value) {
                out.push((Timeframe::H8, value));
            }
        }

        if let Some(ref value) = self.h12 {
            if f(value) {
                out.push((Timeframe::H12, value));
            }
        }

        if let Some(ref value) = self.d1 {
            if f(value) {
                out.push((Timeframe::D1, value));
            }
        }

        if let Some(ref value) = self.d3 {
            if f(value) {
                out.push((Timeframe::D3, value));
            }
        }

        if let Some(ref value) = self.w1 {
            if f(value) {
                out.push((Timeframe::W1, value));
            }
        }

        if let Some(ref value) = self.mm1 {
            if f(value) {
                out.push((Timeframe::MM1, value));
            }
        }

        out
    }

    pub fn reduce<F, U>(&self, f: F) -> Result<TimeSet<U>, TimeframeError>
    where
        F: Fn(&T) -> U,
        U: Clone + Send + Sync,
    {
        Ok(TimeSet {
            mm1: self.mm1.as_ref().map(&f),
            w1:  self.w1.as_ref().map(&f),
            d3:  self.d3.as_ref().map(&f),
            d1:  self.d1.as_ref().map(&f),
            h12: self.h12.as_ref().map(&f),
            h8:  self.h8.as_ref().map(&f),
            h6:  self.h6.as_ref().map(&f),
            h4:  self.h4.as_ref().map(&f),
            h2:  self.h2.as_ref().map(&f),
            h1:  self.h1.as_ref().map(&f),
            m30: self.m30.as_ref().map(&f),
            m15: self.m15.as_ref().map(&f),
            m5:  self.m5.as_ref().map(&f),
            m3:  self.m3.as_ref().map(&f),
            m1:  self.m1.as_ref().map(&f),
            s1:  self.s1.as_ref().map(&f),
        })
    }

    pub fn fold<F, U>(&self, f: F) -> Result<TimeSet<U>, TimeframeError>
    where
        F: Fn(&T) -> U,
        U: Clone + Send + Sync,
    {
        Ok(TimeSet {
            mm1: self.mm1.as_ref().map(&f),
            w1:  self.w1.as_ref().map(&f),
            d3:  self.d3.as_ref().map(&f),
            d1:  self.d1.as_ref().map(&f),
            h12: self.h12.as_ref().map(&f),
            h8:  self.h8.as_ref().map(&f),
            h6:  self.h6.as_ref().map(&f),
            h4:  self.h4.as_ref().map(&f),
            h2:  self.h2.as_ref().map(&f),
            h1:  self.h1.as_ref().map(&f),
            m30: self.m30.as_ref().map(&f),
            m15: self.m15.as_ref().map(&f),
            m5:  self.m5.as_ref().map(&f),
            m3:  self.m3.as_ref().map(&f),
            m1:  self.m1.as_ref().map(&f),
            s1:  self.s1.as_ref().map(&f),
        })
    }

    /*
    pub fn iter(&self) -> impl Iterator<Item = &(Timeframe, &T)>
    where
        T: Clone + Send + Sync,
    {
        let mut out = Vec::<(Timeframe, &T)>::with_capacity(16);

        if let Some(ref value) = self.mm1 {
            out.push((Timeframe::MM1, value));
        }

        if let Some(ref value) = self.w1 {
            out.push((Timeframe::W1, value));
        }

        if let Some(ref value) = self.d3 {
            out.push((Timeframe::D3, value));
        }

        if let Some(ref value) = self.d1 {
            out.push((Timeframe::D1, value));
        }

        if let Some(ref value) = self.h12 {
            out.push((Timeframe::H12, value));
        }

        if let Some(ref value) = self.h8 {
            out.push((Timeframe::H8, value));
        }

        if let Some(ref value) = self.h6 {
            out.push((Timeframe::H6, value));
        }

        if let Some(ref value) = self.h4 {
            out.push((Timeframe::H4, value));
        }

        if let Some(ref value) = self.h2 {
            out.push((Timeframe::H2, value));
        }

        if let Some(ref value) = self.h1 {
            out.push((Timeframe::H1, value));
        }

        if let Some(ref value) = self.m30 {
            out.push((Timeframe::M30, value));
        }

        if let Some(ref value) = self.m15 {
            out.push((Timeframe::M15, value));
        }

        if let Some(ref value) = self.m5 {
            out.push((Timeframe::M5, value));
        }

        if let Some(ref value) = self.m3 {
            out.push((Timeframe::M3, value));
        }

        if let Some(ref value) = self.m1 {
            out.push((Timeframe::M1, value));
        }

        if let Some(ref value) = self.s1 {
            out.push((Timeframe::S1, value));
        }

        out.into_iter()
    }
     */

    pub fn iter(&self) -> impl Iterator<Item = (Timeframe, &T)> {
        IntoIterator::into_iter([
            (Timeframe::S1, &self.s1),
            (Timeframe::M1, &self.m1),
            (Timeframe::M3, &self.m3),
            (Timeframe::M5, &self.m5),
            (Timeframe::M15, &self.m15),
            (Timeframe::M30, &self.m30),
            (Timeframe::H1, &self.h1),
            (Timeframe::H2, &self.h2),
            (Timeframe::H4, &self.h4),
            (Timeframe::H6, &self.h6),
            (Timeframe::H8, &self.h8),
            (Timeframe::H12, &self.h12),
            (Timeframe::D1, &self.d1),
            (Timeframe::D3, &self.d3),
            (Timeframe::W1, &self.w1),
            (Timeframe::MM1, &self.mm1),
        ])
        .filter_map(|option| option.1.as_ref().map(|value| (option.0, value)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Timeframe, &mut T)> {
        IntoIterator::into_iter([
            (Timeframe::S1, &mut self.s1),
            (Timeframe::M1, &mut self.m1),
            (Timeframe::M3, &mut self.m3),
            (Timeframe::M5, &mut self.m5),
            (Timeframe::M15, &mut self.m15),
            (Timeframe::M30, &mut self.m30),
            (Timeframe::H1, &mut self.h1),
            (Timeframe::H2, &mut self.h2),
            (Timeframe::H4, &mut self.h4),
            (Timeframe::H6, &mut self.h6),
            (Timeframe::H8, &mut self.h8),
            (Timeframe::H12, &mut self.h12),
            (Timeframe::D1, &mut self.d1),
            (Timeframe::D3, &mut self.d3),
            (Timeframe::W1, &mut self.w1),
            (Timeframe::MM1, &mut self.mm1),
        ])
        .filter_map(|option| option.1.as_mut().map(|value| (option.0, value)))
    }

    pub fn apply<F>(&mut self, f: F)
    where
        F: Fn(&mut T),
    {
        if let Some(value) = self.mm1.as_mut() {
            f(value);
        }

        if let Some(value) = self.w1.as_mut() {
            f(value);
        }

        if let Some(value) = self.d3.as_mut() {
            f(value);
        }

        if let Some(value) = self.d1.as_mut() {
            f(value);
        }

        if let Some(value) = self.h12.as_mut() {
            f(value);
        }

        if let Some(value) = self.h8.as_mut() {
            f(value);
        }

        if let Some(value) = self.h6.as_mut() {
            f(value);
        }

        if let Some(value) = self.h4.as_mut() {
            f(value);
        }

        if let Some(value) = self.h2.as_mut() {
            f(value);
        }

        if let Some(value) = self.h1.as_mut() {
            f(value);
        }

        if let Some(value) = self.m30.as_mut() {
            f(value);
        }

        if let Some(value) = self.m15.as_mut() {
            f(value);
        }

        if let Some(value) = self.m5.as_mut() {
            f(value);
        }

        if let Some(value) = self.m3.as_mut() {
            f(value);
        }

        if let Some(value) = self.m1.as_mut() {
            f(value);
        }

        if let Some(value) = self.s1.as_mut() {
            f(value);
        }
    }
}

impl<'a, T> IntoIterator for &'a TimeSet<T>
where
    T: Clone + Send + Sync,
{
    type Item = (Timeframe, Option<&'a T>);
    type IntoIter = std::array::IntoIter<Self::Item, 16>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIterator::into_iter([
            (Timeframe::MM1, self.mm1.as_ref()),
            (Timeframe::W1, self.w1.as_ref()),
            (Timeframe::D3, self.d3.as_ref()),
            (Timeframe::D1, self.d1.as_ref()),
            (Timeframe::H12, self.h12.as_ref()),
            (Timeframe::H8, self.h8.as_ref()),
            (Timeframe::H6, self.h6.as_ref()),
            (Timeframe::H4, self.h4.as_ref()),
            (Timeframe::H2, self.h2.as_ref()),
            (Timeframe::H1, self.h1.as_ref()),
            (Timeframe::M30, self.m30.as_ref()),
            (Timeframe::M15, self.m15.as_ref()),
            (Timeframe::M5, self.m5.as_ref()),
            (Timeframe::M3, self.m3.as_ref()),
            (Timeframe::M1, self.m1.as_ref()),
            (Timeframe::S1, self.s1.as_ref()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use crate::timeset::TimeSet;
    use itertools::Itertools;
    use rand::{rngs::OsRng, thread_rng, Rng};
    use std::time::Instant;

    #[test]
    fn test_timeset_map() {
        let a = 1;
        let b = 2;

        let it = Instant::now();
        let mut ts = TimeSet {
            s1:  Some(1),
            m1:  Some(2),
            m3:  Some(3),
            m5:  Some(4),
            m15: Some(5),
            m30: Some(6),
            h1:  Some(7),
            h2:  Some(8),
            h4:  Some(9),
            h6:  Some(10),
            h8:  Some(11),
            h12: Some(12),
            d1:  Some(13),
            d3:  Some(14),
            w1:  Some(15),
            mm1: Some(16),
        };
        println!("{:#?}", it.elapsed());

        for i in 0..10000 {
            let y = OsRng.gen_range(1..1000000);
            let a = y / 2;
            let b = a * y / 2;

            let it = Instant::now();
            let mapped_ts = ts.map(|x| x * OsRng.gen_range(1..1000) / a * b * y);
            println!("{:#?}", it.elapsed());
        }
    }

    #[test]
    fn test_timeset_filter() {
        let a = 1;
        let b = 2;

        let mut ts = TimeSet {
            s1:  Some(1),
            m1:  Some(2),
            m3:  Some(3),
            m5:  Some(4),
            m15: Some(5),
            m30: Some(6),
            h1:  Some(7),
            h2:  Some(8),
            h4:  Some(9),
            h6:  Some(10),
            h8:  Some(11),
            h12: Some(12),
            d1:  Some(13),
            d3:  Some(14),
            w1:  Some(15),
            mm1: Some(16),
        };

        for i in 0..1000 {
            let a = OsRng.gen_range(1..1000);
            let b = a / 2 + a % 3;

            let mapped_ts = ts.map(|x| x * OsRng.gen_range(1..1000) + a * b);

            let it = Instant::now();
            let filtered_ts = mapped_ts.filter(|x| x % 16 == 0);
            println!("{:#?}", it.elapsed());

            if !filtered_ts.is_empty() {
                println!("{:#?}", filtered_ts);
            }
        }
    }

    #[test]
    fn test_timeset_into_iter() {
        let a = 1;
        let b = 2;

        let mut ts = TimeSet {
            s1:  Some(1),
            m1:  Some(2),
            m3:  Some(3),
            m5:  Some(4),
            m15: Some(5),
            m30: Some(6),
            h1:  Some(7),
            h2:  Some(8),
            h4:  Some(9),
            h6:  Some(10),
            h8:  Some(11),
            h12: Some(12),
            d1:  Some(13),
            d3:  Some(14),
            w1:  Some(15),
            mm1: Some(16),
        };

        for i in 0..1000 {
            let a = OsRng.gen_range(1..1000);
            let b = a / 2 + a % 3;

            let mapped_ts = ts.map(|x| x * OsRng.gen_range(1..1000) + a * b);

            let it = Instant::now();
            let mapped_ts = ts
                .into_iter()
                .filter_map(|(timeframe, x)| x.map(|x| x * OsRng.gen_range(1..1000) + a * b))
                .collect_vec();
            println!("{:#?}", it.elapsed());

            println!("{:#?}", mapped_ts);
        }
    }

    #[test]
    fn test_timeset_iter() {
        let a = 1;
        let b = 2;

        let mut ts = TimeSet {
            s1:  Some(1),
            m1:  Some(2),
            m3:  Some(3),
            m5:  Some(4),
            m15: None,
            m30: Some(6),
            h1:  Some(7),
            h2:  Some(8),
            h4:  Some(9),
            h6:  None,
            h8:  None,
            h12: Some(12),
            d1:  Some(13),
            d3:  None,
            w1:  Some(15),
            mm1: Some(16),
        };

        for i in 0..1000 {
            let a = OsRng.gen_range(1..1000);
            let b = a / 2 + a % 3;

            // let mapped_ts = ts.map(|x| x * OsRng.gen_range(1..1000) + a * b);

            let it = Instant::now();
            let mapped_ts = ts
                .iter()
                .map(|(timeframe, x)| (timeframe, x * OsRng.gen_range(1..1000) + a * b))
                .filter(|(timeframe, x)| x % 27 == 0)
                .collect_vec();
            println!("{:#?}", it.elapsed());
            println!("{:#?}", mapped_ts);

            /*
            let it = Instant::now();
            let timeframes = ts.iter().map(|(timeframe, _)| timeframe).collect_vec();
            println!("{:#?}", it.elapsed());
            println!("{:#?}", timeframes);
             */
        }
    }

    #[test]
    fn test_timeset_apply() {
        let a = 1;
        let b = 2;

        let it = Instant::now();
        let mut ts = TimeSet {
            s1:  Some(1u128),
            m1:  Some(2),
            m3:  Some(3),
            m5:  Some(4),
            m15: Some(5),
            m30: Some(6),
            h1:  Some(7),
            h2:  Some(8),
            h4:  Some(9),
            h6:  Some(10),
            h8:  Some(11),
            h12: Some(12),
            d1:  Some(13),
            d3:  Some(14),
            w1:  Some(15),
            mm1: Some(16),
        };
        println!("{:#?}", it.elapsed());

        for i in 0..1000 {
            ts.apply(|x| {
                *x = *x * OsRng.gen_range(1..1000000);
            });
        }

        let it = Instant::now();
        for i in 0..10 {
            ts.apply(|x| {
                *x = *x * OsRng.gen_range(1..1000000);
            });
        }
        println!("{:#?}", it.elapsed());
    }
}
