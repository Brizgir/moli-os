//! The orders to the account, one at a time and not too close: Amazon
//! answers `429 Rate exceeded` to a few orders in a few seconds (three
//! volume settings while a slider is dragged). The orders wait here, at
//! least [`SPACING`] apart; several values for the same point keep only the
//! last one; a 429 is tried again once after [`BACKOFF`], and only a second
//! refusal reaches the person.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;

/// The least time between two orders.
pub(crate) const SPACING: Duration = Duration::from_secs(1);
/// After a 429, before the one retry.
pub(crate) const BACKOFF: Duration = Duration::from_secs(2);

/// Orders that wait, in their order, by the point they write (`T`: the
/// order itself).
#[derive(Debug)]
pub(crate) struct Queue<T> {
    pending: VecDeque<(String, T)>,
    /// When the last order left.
    last: Option<Instant>,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            last: None,
        }
    }
}

impl<T> Queue<T> {
    /// Adds an order for `point`; the order it supersedes (same point,
    /// still waiting), if any, comes back to be answered.
    pub(crate) fn push(&mut self, point: String, order: T) -> Option<T> {
        let old = self
            .pending
            .iter()
            .position(|(p, _)| *p == point)
            .and_then(|i| self.pending.remove(i))
            .map(|(_, o)| o);
        self.pending.push_back((point, order));
        old
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// When the next order may leave: now when nothing left lately.
    pub(crate) fn ready_at(&self, now: Instant) -> Instant {
        self.last.map_or(now, |t| (t + SPACING).max(now))
    }

    /// The next order, once [`Self::ready_at`] has come.
    pub(crate) fn pop(&mut self, now: Instant) -> Option<T> {
        let (_, order) = self.pending.pop_front()?;
        self.last = Some(now);
        Some(order)
    }

    /// An order went (the retry too): the next waits from now.
    pub(crate) fn sent(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_leave_one_second_apart() {
        let mut q: Queue<i64> = Queue::default();
        let t0 = Instant::now();
        assert!(q.is_empty());
        assert_eq!(q.ready_at(t0), t0);
        assert_eq!(q.push("salon/volume".into(), 20), None);
        assert_eq!(q.push("salon/say".into(), 1), None);
        assert_eq!(q.pop(t0), Some(20));
        assert_eq!(q.ready_at(t0), t0 + SPACING);
        assert_eq!(
            q.ready_at(t0 + Duration::from_secs(5)),
            t0 + Duration::from_secs(5)
        );
        assert_eq!(q.pop(t0 + SPACING), Some(1));
        assert!(q.is_empty());
        assert_eq!(q.pop(t0), None);
        q.sent(t0 + Duration::from_secs(9));
        assert_eq!(q.ready_at(t0), t0 + Duration::from_secs(10));
    }

    #[test]
    fn several_values_for_one_point_keep_the_last() {
        let mut q: Queue<i64> = Queue::default();
        assert_eq!(q.push("salon/volume".into(), 20), None);
        assert_eq!(q.push("cuisine/volume".into(), 30), None);
        // Superseded: the old order comes back, the new one keeps its place
        // at the end.
        assert_eq!(q.push("salon/volume".into(), 25), Some(20));
        assert_eq!(q.push("salon/volume".into(), 28), Some(25));
        let t = Instant::now();
        assert_eq!(q.pop(t), Some(30));
        assert_eq!(q.pop(t), Some(28));
        assert!(q.is_empty());
    }
}
