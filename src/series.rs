use crate::sliding_window::SlidingWindow;
use serde_derive::{Deserialize, Serialize};
use std::ops::{Index, IndexMut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series<T>(SlidingWindow<T>);

impl<T> Index<usize> for Series<T>
where
    T: std::fmt::Debug + Clone,
{
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).expect("Index out of bounds")
    }
}

impl<T> IndexMut<usize> for Series<T>
where
    T: std::fmt::Debug + Clone,
{
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        self.get_mut(index).expect("Index out of bounds")
    }
}

impl<T> Series<T>
where
    T: std::fmt::Debug + Clone,
{
    pub fn new(capacity: usize) -> Self {
        Series(SlidingWindow::new(capacity))
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    #[inline]
    pub fn push(&mut self, item: T) -> Option<T> {
        self.0.push(item)
    }

    #[inline]
    pub fn head(&self) -> Option<&T> {
        self.0.head()
    }

    #[inline]
    pub fn head_mut(&mut self) -> Option<&mut T> {
        self.0.head_mut()
    }

    #[inline]
    pub fn tail(&self) -> Option<&T> {
        self.0.tail()
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.0.get_rev(index)
    }

    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.0.get_rev_mut(index)
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.0.iter_rev()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        self.0.iter_rev_mut()
    }

    #[inline]
    pub fn iter_rev(&self) -> impl Iterator<Item = &T> + '_ {
        self.0.iter()
    }

    #[inline]
    pub fn iter_rev_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        self.0.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_series() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        assert_eq!(series.head(), Some(&5));
        assert_eq!(series.tail(), Some(&1));

        let expected = vec![&5, &4, &3, &2, &1];
        let actual: Vec<_> = series.iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_index() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        assert_eq!(series[0], 5);
        assert_eq!(series[1], 4);
        assert_eq!(series[4], 1);
    }

    #[test]
    fn test_index_mut() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        series[0] = 10;
        series[1] = 20;
        series[4] = 50;

        assert_eq!(series[0], 10);
        assert_eq!(series[1], 20);
        assert_eq!(series[4], 50);
    }

    #[test]
    fn test_iter() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        let expected = vec![&5, &4, &3, &2, &1];
        let actual: Vec<_> = series.iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_iter_mut() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        for item in series.iter_mut() {
            *item *= 10;
        }

        let expected = vec![&50, &40, &30, &20, &10];
        let actual: Vec<_> = series.iter().collect();
        assert_eq!(actual, expected);
    }

    #[test]
    #[should_panic]
    fn test_index_out_of_bounds() {
        let series = Series::<u64>::new(5);
        let _ = series[5];
    }

    #[test]
    #[should_panic]
    fn test_index_mut_out_of_bounds() {
        let mut series = Series::new(5);
        series[5] = 10;
    }

    #[test]
    fn test_get() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        assert_eq!(series.get(0), Some(&5));
        assert_eq!(series.get(1), Some(&4));
        assert_eq!(series.get(4), Some(&1));
        assert_eq!(series.get(5), None);
    }

    #[test]
    fn test_get_range() {
        let mut series = Series::new(5);

        series.push(1);
        series.push(2);
        series.push(3);
        series.push(4);
        series.push(5);

        let expected = vec![&4, &3, &2];
        let actual: Vec<_> = (1..4).map(|i| series.get(i).unwrap()).collect();
        assert_eq!(actual, expected);

        assert_eq!(series.head(), Some(&5));
        assert_eq!(series.head_mut(), Some(&mut 5));
        assert_eq!(series.tail(), Some(&1));
    }
}
