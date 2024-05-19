pub struct SlidingWindowIter<'a, T> {
    buffer: &'a [Option<T>], // Direct reference to the slice of the buffer
    start: usize,
    end: usize,
    current: usize,
}

impl<'a, T> Iterator for SlidingWindowIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current == self.end {
            None
        } else {
            // IMPORTANT:
            // The unsafe block assumes that all elements from `start` to `end`
            // are initialized (Some(T)). This is based on the invariants upheld by the
            // SlidingWindow's push operation.
            let item = unsafe {
                // Directly access the element, bypassing the Option's safety checks.
                // This is safe because we know the element at `current` must be Some(T)
                // due to the constraints of how the SlidingWindow is filled and iterated.
                &*self.buffer[self.current].as_ref().unwrap_unchecked()
            };

            self.current = (self.current + 1) % self.buffer.len();
            Some(item)
        }
    }
}

pub struct SlidingWindowIterRev<'a, T> {
    buffer: &'a [Option<T>],
    end: usize,     // Represents the logical end of iteration (inclusive).
    current: isize, // Use isize to support wrapping around at 0.
}

impl<'a, T> Iterator for SlidingWindowIterRev<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current < 0 {
            None
        } else {
            let item = unsafe {
                // Safety guaranteed by buffer invariants.
                self.buffer
                    .get(self.current as usize)
                    .and_then(Option::as_ref)
                    .unwrap_unchecked()
            };

            // Decrement current, wrap around if necessary.
            self.current = if self.current == 0 {
                self.end as isize
            } else {
                self.current - 1
            };
            Some(item)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd)]
pub struct SlidingWindow<T> {
    buffer: Box<[Option<T>]>,
    capacity: usize,
    head: usize,
    size: usize,
}

impl<T> SlidingWindow<T>
where
    T: std::fmt::Debug + Clone,
{
    pub fn new(capacity: usize) -> Self {
        SlidingWindow {
            buffer: vec![None; capacity].into_boxed_slice(),
            capacity,
            head: 0,
            size: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.size
    }

    #[inline]
    pub fn push(&mut self, item: T) -> Option<T> {
        let idx = (self.head + self.size) % self.capacity;
        let replaced_value = self.buffer[idx].replace(item);

        if self.size < self.capacity {
            self.size += 1;
        } else {
            self.head = (self.head + 1) % self.capacity;
        };

        replaced_value
    }

    #[inline]
    pub fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            None
        } else {
            let idx = (self.head + self.size - 1) % self.capacity;
            self.size -= 1;
            self.buffer[idx].take()
        }
    }

    #[inline]
    pub fn head(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            let idx = (self.head + self.size - 1) % self.capacity;
            self.buffer[idx].as_ref()
        }
    }

    #[inline]
    pub fn head_mut(&mut self) -> Option<&mut T> {
        if self.is_empty() {
            None
        } else {
            let idx = (self.head + self.size - 1) % self.capacity;
            self.buffer[idx].as_mut()
        }
    }

    #[inline]
    pub fn prev(&self) -> Option<&T> {
        if self.size < 2 {
            None
        } else {
            let idx = (self.head + self.size - 2) % self.capacity;
            self.buffer[idx].as_ref()
        }
    }

    #[inline]
    pub fn tail(&self) -> Option<&T> {
        if self.is_empty() {
            None
        } else {
            self.buffer[self.head].as_ref()
        }
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.size {
            None
        } else {
            let idx = (self.head + index) % self.capacity;
            Some(self.buffer[idx].as_ref().unwrap())
        }
    }

    /// Returns a reference to the element at the given reverse index.
    pub fn get_rev(&self, reverse_index: usize) -> Option<&T> {
        if reverse_index >= self.size {
            None
        } else {
            let idx = (self.head + self.size - 1 - reverse_index + self.capacity) % self.capacity;
            self.buffer[idx].as_ref()
        }
    }

    pub fn get_rev_mut(&mut self, reverse_index: usize) -> Option<&mut T> {
        if reverse_index >= self.size {
            None
        } else {
            let idx = (self.head + self.size - 1 - reverse_index + self.capacity) % self.capacity;
            self.buffer[idx].as_mut()
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        let mut index = 0;
        std::iter::from_fn(move || {
            if index >= self.size {
                None
            } else {
                let current = (self.head + index) % self.capacity;
                index += 1;
                // Safe to unwrap because we know these indexes are within the bounds of actual data
                unsafe { Some(&*self.buffer[current].as_ref().unwrap_unchecked()) }
            }
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        let mut index = 0;
        let capacity = self.capacity;
        let head = self.head;
        let size = self.size;
        std::iter::from_fn(move || {
            if index < size {
                let current = (head + index) % capacity;
                index += 1;
                // IMPORTANT:
                // The unsafe block here assumes that the index calculation keeps
                // us within bounds and that we are not violating Rust's aliasing rules
                // since we ensure each element is only yielded once per iteration.
                unsafe {
                    let ptr = self.buffer.as_mut_ptr().add(current) as *mut Option<T>;
                    Some(ptr.as_mut().unwrap().as_mut().unwrap_unchecked())
                }
            } else {
                None
            }
        })
    }

    pub fn iter_rev(&self) -> impl Iterator<Item = &T> + '_ {
        let mut index = 0;
        std::iter::from_fn(move || {
            if index < self.size {
                let current = (self.head + self.size - 1 - index + self.capacity) % self.capacity;
                index += 1;
                // Safe to unwrap because we know these indexes are within the bounds of actual data
                unsafe { Some(&*self.buffer[current].as_ref().unwrap_unchecked()) }
            } else {
                None
            }
        })
    }

    pub fn iter_rev_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        let mut index = 0;
        let capacity = self.capacity;
        let head = self.head;
        let size = self.size;
        std::iter::from_fn(move || {
            if index < size {
                let current = (head + size - 1 - index + capacity) % capacity;
                index += 1;
                // IMPORTANT: Same safety assumptions as in iter_mut.
                unsafe {
                    let ptr = self.buffer.as_mut_ptr().add(current) as *mut Option<T>;
                    Some(ptr.as_mut().unwrap().as_mut().unwrap_unchecked())
                }
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use std::time::Instant;

    #[test]
    fn test_sliding_window_push() {
        let mut buffer = SlidingWindow::new(5);

        buffer.push(1);
        buffer.push(2);
        buffer.push(3);
        buffer.push(4);
        buffer.push(5);

        assert_eq!(buffer.len(), 5);
        assert_eq!(buffer.capacity(), 5);
        assert_eq!(buffer.head(), Some(&5));
        assert_eq!(buffer.prev(), Some(&4));
        assert_eq!(buffer.tail(), Some(&1));
    }

    #[test]
    fn test_ring_get_and_get_reverse() {
        let mut buffer = SlidingWindow::new(5);

        buffer.push(1);
        buffer.push(2);
        buffer.push(3);
        buffer.push(4);
        buffer.push(5);

        assert_eq!(buffer.get(0), Some(&1));
        assert_eq!(buffer.get(1), Some(&2));
        assert_eq!(buffer.get(2), Some(&3));
        assert_eq!(buffer.get(3), Some(&4));
        assert_eq!(buffer.get(4), Some(&5));
        assert_eq!(buffer.get(5), None);

        assert_eq!(buffer.get_rev(0), Some(&5));
        assert_eq!(buffer.get_rev(1), Some(&4));
        assert_eq!(buffer.get_rev(2), Some(&3));
        assert_eq!(buffer.get_rev(3), Some(&2));
        assert_eq!(buffer.get_rev(4), Some(&1));
        assert_eq!(buffer.get_rev(5), None);
    }

    #[test]
    fn test_ring_head_and_tail() {
        let mut buffer = SlidingWindow::new(5);

        buffer.push(1);
        buffer.push(2);
        buffer.push(3);
        buffer.push(4);
        buffer.push(5);

        assert_eq!(buffer.head(), Some(&5));
        assert_eq!(buffer.tail(), Some(&1));

        buffer.pop();
        assert_eq!(buffer.head(), Some(&4));
        assert_eq!(buffer.tail(), Some(&1));

        buffer.pop();
        assert_eq!(buffer.head(), Some(&3));
        assert_eq!(buffer.tail(), Some(&1));

        buffer.pop();
        assert_eq!(buffer.head(), Some(&2));
        assert_eq!(buffer.tail(), Some(&1));

        buffer.pop();
        assert_eq!(buffer.head(), Some(&1));
        assert_eq!(buffer.tail(), Some(&1));

        buffer.pop();
        assert_eq!(buffer.head(), None);
        assert_eq!(buffer.tail(), None);
    }

    #[test]
    fn test_sliding_window() {
        let mut buffer = SlidingWindow::new(1500);

        println!("push():");
        println!("-------------------------------");
        for _ in 0..5 {
            let it = Instant::now();
            for i in 1..=1000 {
                buffer.push(i);
            }
            print!("{:#?} ", it.elapsed());

            let it = Instant::now();
            for i in 1..=10000 {
                buffer.push(i);
            }
            print!("{:#?} ", it.elapsed());

            let it = Instant::now();
            for i in 1..=100000 {
                buffer.push(i);
            }
            println!("{:#?}", it.elapsed());
        }
    }

    #[test]
    fn test_sliding_window_iter() {
        println!("iter():");
        let mut buffer = SlidingWindow::new(10);

        for i in 1..=15 {
            buffer.push(i);
        }

        for &item in buffer.iter() {
            println!("{}", item);
        }
    }

    #[test]
    fn test_sliding_window_rev() {
        println!("iter_rev():");
        let mut buffer = SlidingWindow::new(10);

        for i in 1..=15 {
            buffer.push(i);
        }

        for &item in buffer.iter_rev() {
            println!("{}", item);
        }
    }

    #[test]
    fn test_iter_performance() {
        let mut buffer = SlidingWindow::new(10000);
        for i in 0..100000 {
            buffer.push(i);
        }

        let start = Instant::now();
        for x in buffer.iter() {
            let _ = x * 5;
        }
        let duration = start.elapsed();

        println!("Time taken for iter(): {:?}", duration);
    }

    #[test]
    fn test_iter_rev_performance() {
        let mut buffer = SlidingWindow::new(10000);
        for i in 0..100000 {
            buffer.push(i);
        }

        let start = Instant::now();
        for x in buffer.iter_rev() {
            let _ = x * 5;
        }
        let duration = start.elapsed();

        println!("Time taken for iter_rev(): {:?}", duration);
    }

    // ----------------------------------------------

    #[test]
    fn test_iter_mut() {
        let mut buffer = SlidingWindow::new(5);
        for i in 1..=5 {
            buffer.push(i);
        }

        // Increment each element in the buffer by 1 using iter_mut.
        buffer.iter_mut().for_each(|x| *x += 1);

        // Collect the buffer's contents to verify the mutation.
        let contents: Vec<_> = buffer.iter().cloned().collect();
        assert_eq!(contents, vec![2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_iter_rev_mut() {
        let mut buffer = SlidingWindow::new(5);
        for i in 1..=5 {
            buffer.push(i);
        }

        // Decrement each element in the buffer by 1 using iter_rev_mut.
        buffer.iter_rev_mut().for_each(|x| *x -= 1);

        // Collect the buffer's contents to verify the mutation and order.
        let contents: Vec<_> = buffer.iter().cloned().collect();
        assert_eq!(contents, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn test_sliding_window_behavior() {
        let mut buffer = SlidingWindow::new(5);

        // Initially fill the buffer.
        for i in 1..=5 {
            buffer.push(i);
        }

        // These pushes will slide the window, discarding the oldest elements.
        buffer.push(6); // Discarding 1, buffer is now [2, 3, 4, 5, 6]
        buffer.push(7); // Discarding 2, buffer becomes [3, 4, 5, 6, 7]

        // Increment each element in the buffer by 10.
        buffer.iter_mut().for_each(|x| *x += 10);

        // Collect the buffer's contents to verify.
        let contents: Vec<_> = buffer.iter().cloned().collect();

        // The expected state after sliding and mutation.
        assert_eq!(contents, vec![13, 14, 15, 16, 17]);
    }

    #[test]
    fn test_empty_buffer() {
        let mut buffer: SlidingWindow<i32> = SlidingWindow::new(5);

        {
            // Verify iter_mut on an empty buffer.
            let mut iter_mut = buffer.iter_mut();
            assert!(iter_mut.next().is_none());
        }

        {
            // Verify iter_rev_mut on an empty buffer.
            let mut iter_rev_mut = buffer.iter_rev_mut();
            assert!(iter_rev_mut.next().is_none());
        }
    }
}
