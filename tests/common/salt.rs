//! Both ends of a salt pipe, as a test drives them.
//!
//! A `Producer` offers a queue into an `in` pipe of the module under test and a
//! `Consumer` drains one of its `out` pipes, each following the protocol the
//! compiled module does: gray-coded two-bit salts over a buffer that holds two.
//! `Lcg` makes irregular willingness reproducible, and `Steps` counts how many
//! entries the module took from an input.
#![allow(dead_code)]

use super::{Circuit, mask};

/// The entry a salt names: bit 0 of its gray-code index.
pub fn index(salt: u128) -> u128 {
    (salt ^ (salt >> 1)) & 1
}

/// A salt after one transfer.
pub fn step(salt: u128) -> u128 {
    salt ^ if index(salt) == 0 { 1 } else { 2 }
}

/// The producing end of an `in` pipe, offering `queue` in order.
pub struct Producer {
    pipe: &'static str,
    width: u32,
    wsalt: u128,
    data: u128,
    pub queue: std::collections::VecDeque<u128>,
}

impl Producer {
    pub fn new(pipe: &'static str, width: u32, items: impl IntoIterator<Item = u128>) -> Self {
        Producer { pipe, width, wsalt: 0, data: 0, queue: items.into_iter().collect() }
    }

    /// Writes the next item if `willing` and the pipe has room.
    pub fn offer(&mut self, c: &mut Circuit, willing: bool) {
        let rsalt = c.out(&format!("{}_rsalt", self.pipe));
        let full = self.wsalt == (!rsalt & 3);
        if full || !willing {
            return;
        }
        let Some(item) = self.queue.pop_front() else { return };
        let shift = index(self.wsalt) as u32 * self.width;
        self.data = (self.data & !(mask(self.width) << shift)) | (item << shift);
        self.wsalt = step(self.wsalt);
        c.set(&format!("{}_data", self.pipe), self.data);
        c.set(&format!("{}_wsalt", self.pipe), self.wsalt);
    }
}

/// The consuming end of an `out` pipe.
pub struct Consumer {
    pipe: &'static str,
    width: u32,
    pub rsalt: u128,
}

impl Consumer {
    pub fn new(pipe: &'static str, width: u32) -> Self {
        Consumer { pipe, width, rsalt: 0 }
    }

    /// Takes the entry on offer, if there is one and the consumer is `willing`.
    pub fn take(&mut self, c: &mut Circuit, willing: bool) -> Option<u128> {
        let has_item = c.out(&format!("{}_wsalt", self.pipe)) != self.rsalt;
        if !has_item || !willing {
            return None;
        }
        let shift = index(self.rsalt) as u32 * self.width;
        let item = (c.out(&format!("{}_data", self.pipe)) >> shift) & mask(self.width);
        self.rsalt = step(self.rsalt);
        c.set(&format!("{}_rsalt", self.pipe), self.rsalt);
        Some(item)
    }
}

/// A fixed pseudo-random stream, so a failure reproduces.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn chance(&mut self, percent: u64) -> bool {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % 100 < percent
    }
}

/// Counts steps of an `rsalt` the module drives, one call per cycle.
pub struct Steps {
    port: String,
    last: u128,
    pub count: usize,
}

impl Steps {
    pub fn new(pipe: &str) -> Self {
        Steps::port(format!("{}_rsalt", pipe))
    }

    /// Any salt the module drives -- `<p>_wsalt` counts what it pushed.
    pub fn port(port: String) -> Self {
        Steps { port, last: 0, count: 0 }
    }

    pub fn watch(&mut self, c: &Circuit) {
        let now = c.out(&self.port);
        if now != self.last {
            // One transfer per cycle at most: one bit of the gray code.
            assert_eq!(step(self.last), now, "`{}` moved by more than one entry", self.port);
            self.count += 1;
            self.last = now;
        }
    }
}
