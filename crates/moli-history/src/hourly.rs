//! Hourly statistics, kept for good once the raw changes are pruned: Home
//! Assistant's long-term statistics (its recorder's `statistics` table).
//! Pure: no clock, no storage.
//!
//! As in Home Assistant, the mean is weighted by time: a value held for 50
//! minutes counts fifty times more than one held for a minute (a plain
//! average of the changes would favour a sensor's noisy moments). Minimum
//! and maximum include the value held when the hour opens. A point offline
//! (`null`) counts for nothing while it is. Only the hours in which a point
//! changed get a row: in between, it held the previous row's `last`.

pub(crate) const HOUR_MS: i64 = 3_600_000;

/// One hour of one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hour {
    /// Start of the hour (ms since the epoch, a multiple of an hour).
    pub start: i64,
    /// `None` when the point had no value (offline) the whole hour.
    pub mean: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The value held when the hour closes (`None`: offline).
    pub last: Option<f64>,
}

impl Hour {
    /// An hour without any change: the held value, all along.
    pub fn held(start: i64, value: Option<f64>) -> Self {
        Self {
            start,
            mean: value,
            min: value,
            max: value,
            last: value,
        }
    }
}

/// The start of the hour holding `ts`.
pub(crate) fn floor(ts: i64) -> i64 {
    ts.div_euclid(HOUR_MS) * HOUR_MS
}

/// Statistics of each hour of `[from, to)` (hour-aligned) in which the
/// point changed, from the value it held at `from` and its changes (sorted
/// by time, all within the range). Time after `until` does not count (the
/// hour still running).
pub(crate) fn hours(
    mut held: Option<f64>,
    changes: &[(i64, Option<f64>)],
    from: i64,
    to: i64,
    until: i64,
) -> Vec<Hour> {
    let mut out: Vec<Hour> = Vec::new();
    let mut rest = changes
        .iter()
        .filter(|(ts, _)| (from..to).contains(ts))
        .peekable();
    while let Some(&&(first, _)) = rest.peek() {
        let start = floor(first);
        let end = (start + HOUR_MS).min(until.max(start));
        let (mut sum, mut covered, mut at) = (0.0, 0_i64, start);
        let (mut min, mut max) = (held, held);
        let mut weigh = |value: Option<f64>, from: i64, to: i64| {
            if let Some(v) = value {
                #[allow(clippy::cast_precision_loss)]
                let span = (to - from).max(0) as f64;
                sum += v * span;
                covered += (to - from).max(0);
            }
        };
        while let Some(&&(ts, value)) = rest.peek() {
            if ts >= start + HOUR_MS {
                break;
            }
            rest.next();
            weigh(held, at, ts.min(end));
            held = value;
            at = ts.min(end);
            if let Some(v) = value {
                min = Some(min.map_or(v, |m: f64| m.min(v)));
                max = Some(max.map_or(v, |m: f64| m.max(v)));
            }
        }
        weigh(held, at, end);
        #[allow(clippy::cast_precision_loss)]
        let mean = if covered > 0 {
            Some(sum / covered as f64)
        } else {
            // Nothing lived yet (a change right now): the value itself.
            held
        };
        out.push(Hour {
            start,
            mean,
            min,
            max,
            last: held,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = HOUR_MS;
    const M: i64 = 60_000;

    #[test]
    fn the_mean_is_weighted_by_time() {
        // 20 for 50 minutes, then 26 for 10 minutes: 21, not 23.
        let hours = hours(Some(20.0), &[(50 * M, Some(26.0))], 0, H, i64::MAX);
        assert_eq!(hours.len(), 1);
        let h = hours[0];
        assert!((h.mean.unwrap() - 21.0).abs() < 1e-9, "{h:?}");
        assert_eq!((h.min, h.max, h.last), (Some(20.0), Some(26.0), Some(26.0)));
    }

    #[test]
    fn only_hours_with_changes_get_a_row_and_values_carry_over() {
        let changes = [(10 * M, Some(1.0)), (3 * H + 30 * M, Some(3.0))];
        let hours = hours(None, &changes, 0, 5 * H, i64::MAX);
        assert_eq!(
            hours.iter().map(|h| h.start).collect::<Vec<_>>(),
            [0, 3 * H]
        );
        // The first hour: nothing known for 10 minutes, then 1.
        assert_eq!(hours[0].mean, Some(1.0));
        // The fourth: 1 (held since the first) for half an hour, then 3.
        assert!((hours[1].mean.unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(hours[1].min, Some(1.0));
    }

    #[test]
    fn offline_time_counts_for_nothing() {
        let changes = [(15 * M, None), (45 * M, Some(10.0))];
        let h = hours(Some(4.0), &changes, 0, H, i64::MAX)[0];
        // 4 for 15 min, offline 30 min, 10 for 15 min.
        assert!((h.mean.unwrap() - 7.0).abs() < 1e-9);
        let gone = hours(Some(4.0), &[(0, None)], 0, H, i64::MAX)[0];
        assert_eq!((gone.mean, gone.last), (None, None));
    }

    #[test]
    fn the_running_hour_counts_until_now() {
        // 0 for 10 minutes, then 6 for 20: the 30 minutes not yet lived do
        // not count.
        let h = hours(Some(0.0), &[(10 * M, Some(6.0))], 0, H, 30 * M)[0];
        assert!((h.mean.unwrap() - 4.0).abs() < 1e-9, "{h:?}");
    }

    #[test]
    fn outside_the_range_is_ignored() {
        assert!(hours(Some(1.0), &[(2 * H, Some(2.0))], 0, H, i64::MAX).is_empty());
        assert_eq!(floor(-1), -H);
        assert_eq!(floor(H + 1), H);
    }
}
