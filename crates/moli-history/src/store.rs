//! SQLite storage. Synchronous: always called from blocking threads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use moli_core::{PointId, Value};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value as Json;

use crate::hourly::{self, HOUR_MS, Hour};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS points (
    id    INTEGER PRIMARY KEY,
    point TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS samples (
    point_id INTEGER NOT NULL REFERENCES points(id),
    ts       INTEGER NOT NULL,           -- ms since the Unix epoch
    kind     TEXT    NOT NULL,           -- b(ool) i(nt) f(loat) t(ext) z(null)
    num      REAL,                       -- bool as 0/1, int, float
    txt      TEXT,
    PRIMARY KEY (point_id, ts)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS samples_ts ON samples(ts);
-- What agents query: one row per recorded change.
CREATE VIEW IF NOT EXISTS history AS
    SELECT p.point AS point, s.ts AS ts, s.kind AS kind, s.num AS num, s.txt AS txt
    FROM samples s JOIN points p ON p.id = s.point_id;
-- Long-term statistics (Home Assistant's): one row per hour in which a
-- numeric point changed, kept for good. In between, it held `last`.
CREATE TABLE IF NOT EXISTS hourly (
    point_id INTEGER NOT NULL REFERENCES points(id),
    hour     INTEGER NOT NULL,           -- ms, start of the hour
    mean     REAL,                       -- weighted by time; null: offline
    min      REAL,
    max      REAL,
    last     REAL,                       -- held when the hour closes
    PRIMARY KEY (point_id, hour)
) WITHOUT ROWID;
CREATE VIEW IF NOT EXISTS history_hourly AS
    SELECT p.point AS point, h.hour AS hour, h.mean AS mean, h.min AS min, h.max AS max,
           h.last AS last
    FROM hourly h JOIN points p ON p.id = h.point_id;
-- Bookkeeping: `hourly_until` (first hour not compiled yet), `pruned_before`.
CREATE TABLE IF NOT EXISTS marks (
    name  TEXT PRIMARY KEY,
    value INTEGER NOT NULL
) WITHOUT ROWID;
";

/// Hours compiled at once: a day (a first start catches up on its month of
/// history a day at a time, never holding the writer long).
const HOURS_PER_PASS: i64 = 24;
/// An hour is compiled once this long has passed since it closed: the
/// recorder hands its changes over in batches.
const SETTLE_MS: i64 = 5 * 60_000;

/// Agents' SQL: time budget per query.
const SQL_BUDGET: Duration = Duration::from_secs(2);
/// Agents' SQL: largest string or blob any expression may build.
const AGENT_MAX_VALUE: i32 = 64 * 1024;
/// Agents' SQL: rough size cap of one answer.
const AGENT_MAX_ANSWER: usize = 1024 * 1024;
/// Same-millisecond samples of one point kept (then the rest is dropped).
const MAX_SAME_MS: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("only a single read-only statement is accepted")]
    NotReadOnly,
    #[error("query took longer than {}s", SQL_BUDGET.as_secs())]
    TooSlow,
    #[error("{0}")]
    Internal(String),
}

/// One bucket of a downsampled numeric series.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bucket {
    /// First sample's time in the bucket.
    pub ts: u64,
    pub avg: f64,
    pub min: f64,
    pub max: f64,
    pub count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Series {
    pub point: String,
    pub from: u64,
    pub to: u64,
    /// Number of changes recorded in the window.
    pub total: u64,
    /// Last value before the window (a step chart starts from it).
    pub before: Option<(u64, Value)>,
    /// First and last changes inside the window (whatever the shape below).
    pub first: Option<(u64, Value)>,
    pub last: Option<(u64, Value)>,
    /// Every change, when there are at most `max_points` of them.
    pub raw: Vec<(u64, Value)>,
    /// Otherwise, numeric points are bucketed…
    pub buckets: Vec<Bucket>,
    /// …and non-numeric ones keep their most recent changes.
    pub truncated: bool,
    /// The buckets come from the hourly statistics: the window reaches
    /// further back than the raw changes are kept.
    pub hourly: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SqlResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Json>>,
    pub truncated: bool,
}

pub struct Store {
    path: PathBuf,
    writer: Mutex<Connection>,
    reader: Mutex<Connection>,
    agents: Mutex<Connection>,
    ids: Mutex<HashMap<String, i64>>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// (kind, num, txt) for storage.
fn encode(value: &Value) -> (&'static str, Option<f64>, Option<&str>) {
    #[allow(clippy::cast_precision_loss)]
    match value {
        Value::Null => ("z", None, None),
        Value::Bool(b) => ("b", Some(f64::from(u8::from(*b))), None),
        Value::Int(i) => ("i", Some(*i as f64), None),
        Value::Float(f) => ("f", Some(*f), None),
        Value::Text(t) => ("t", None, Some(t)),
    }
}

fn decode(kind: &str, num: Option<f64>, txt: Option<String>) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    match (kind, num, txt) {
        ("b", Some(n), _) => Value::Bool(n != 0.0),
        ("i", Some(n), _) => Value::Int(n as i64),
        ("f", Some(n), _) => Value::Float(n),
        ("t", _, Some(t)) => Value::Text(t.into()),
        _ => Value::Null,
    }
}

#[allow(clippy::cast_sign_loss)]
fn ms(ts: i64) -> u64 {
    ts.max(0) as u64
}

fn ts(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, HistoryError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| HistoryError::Internal(e.to_string()))?;
        }
        let writer = Connection::open(path)?;
        writer.pragma_update(None, "journal_mode", "WAL")?;
        writer.pragma_update(None, "synchronous", "NORMAL")?;
        writer.busy_timeout(Duration::from_secs(5))?;
        writer.execute_batch(SCHEMA)?;

        let read_only = || -> Result<Connection, HistoryError> {
            let conn = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            conn.busy_timeout(Duration::from_secs(5))?;
            Ok(conn)
        };
        let reader = read_only()?;
        let agents = read_only()?;
        agents.pragma_update(None, "query_only", true)?;
        // Agents read data; they never attach files, change settings, or
        // hold transactions open (a pinned read snapshot would stall the WAL).
        agents.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Attach { .. }
            | AuthAction::Detach { .. }
            | AuthAction::Pragma { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. } => Authorization::Deny,
            _ => Authorization::Allow,
        }))?;
        // No giant strings or blobs (printf('%.*c', 1e9, 'x'), zeroblob…).
        agents.set_limit(Limit::SQLITE_LIMIT_LENGTH, AGENT_MAX_VALUE)?;
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            reader: Mutex::new(reader),
            agents: Mutex::new(agents),
            ids: Mutex::new(HashMap::new()),
        })
    }

    /// Records changes. `restate`: these rows re-state values already known
    /// (current state after a restart or a lag) — an identical sample at the
    /// same instant is then skipped instead of counted twice.
    pub fn insert(
        &self,
        rows: &[(PointId, Value, u64)],
        restate: bool,
    ) -> Result<(), HistoryError> {
        let mut conn = lock(&self.writer);
        let tx = conn.transaction()?;
        {
            let mut ids = lock(&self.ids);
            let mut upsert = tx.prepare(
                "INSERT INTO points(point) VALUES (?1)
                 ON CONFLICT(point) DO UPDATE SET point = excluded.point RETURNING id",
            )?;
            // Two changes in the same millisecond (two button presses…) are
            // two samples: a collision moves the later one by 1 ms.
            let mut sample = tx.prepare(
                "INSERT INTO samples(point_id, ts, kind, num, txt) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(point_id, ts) DO NOTHING",
            )?;
            for (point, value, at) in rows {
                let id = if let Some(id) = ids.get(point.as_str()) {
                    *id
                } else {
                    let id: i64 = upsert.query_row([point.as_str()], |r| r.get(0))?;
                    ids.insert(point.as_str().to_owned(), id);
                    id
                };
                let (kind, num, txt) = encode(value);
                for at in (ts(*at)..).take(MAX_SAME_MS) {
                    if sample.execute(params![id, at, kind, num, txt])? == 1 || restate {
                        break;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes samples older than `cutoff_ms`, except the latest of each
    /// point before it (a value stable for months must still be known), and
    /// never what the hourly statistics have not taken in yet. Returns how
    /// many.
    pub fn prune(&self, cutoff_ms: u64) -> Result<usize, HistoryError> {
        let conn = lock(&self.writer);
        let compiled = mark(&conn, "hourly_until")?;
        let cutoff = compiled.map_or(ts(cutoff_ms), |c| c.min(ts(cutoff_ms)));
        let removed = conn.execute(
            "DELETE FROM samples WHERE ts < ?1
             AND ts < (SELECT MAX(s.ts) FROM samples s
                       WHERE s.point_id = samples.point_id AND s.ts < ?1)",
            [cutoff],
        )?;
        if mark(&conn, "pruned_before")?.is_none_or(|before| before < cutoff) {
            set_mark(&conn, "pruned_before", cutoff)?;
        }
        Ok(removed)
    }

    /// Compiles the hourly statistics of up to a day of closed hours.
    /// Returns the rows written and whether it has caught up with `now_ms`.
    pub fn compile(&self, now_ms: u64) -> Result<(usize, bool), HistoryError> {
        let mut conn = lock(&self.writer);
        let closed = hourly::floor(ts(now_ms) - SETTLE_MS);
        let from = if let Some(from) = mark(&conn, "hourly_until")? {
            from
        } else {
            // The first time: from the oldest change recorded.
            let oldest: Option<i64> =
                conn.query_row("SELECT MIN(ts) FROM samples", [], |r| r.get(0))?;
            oldest.map_or(closed, hourly::floor).min(closed)
        };
        let to = (from + HOURS_PER_PASS * HOUR_MS).min(closed);
        let tx = conn.transaction()?;
        let mut written = 0;
        if from < to {
            let mut changes: HashMap<i64, Vec<(i64, Option<f64>)>> = HashMap::new();
            let mut numeric: HashMap<i64, bool> = HashMap::new();
            {
                let mut stmt = tx.prepare(
                    "SELECT point_id, ts, kind, num FROM samples
                     WHERE ts >= ?1 AND ts < ?2 ORDER BY point_id, ts",
                )?;
                let mut rows = stmt.query(params![from, to])?;
                while let Some(row) = rows.next()? {
                    let (id, at, kind, num): (i64, i64, String, Option<f64>) =
                        (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
                    *numeric.entry(id).or_default() |= num.is_some();
                    // Text is not a number: it counts like offline.
                    changes
                        .entry(id)
                        .or_default()
                        .push((at, num.filter(|_| kind != "t")));
                }
            }
            let mut before = tx.prepare(
                "SELECT num FROM samples WHERE point_id = ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 1",
            )?;
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO hourly(point_id, hour, mean, min, max, last)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (id, changes) in &changes {
                let held: Option<f64> = before
                    .query_row(params![id, from], |r| r.get(0))
                    .optional()?
                    .flatten();
                if !numeric[id] && held.is_none() {
                    continue;
                }
                for h in hourly::hours(held, changes, from, to, to) {
                    insert.execute(params![id, h.start, h.mean, h.min, h.max, h.last])?;
                    written += 1;
                }
            }
        }
        set_mark(&tx, "hourly_until", to.max(from))?;
        tx.commit()?;
        Ok((written, to >= closed))
    }

    pub fn series(
        &self,
        point: &str,
        from: u64,
        to: u64,
        max_points: usize,
    ) -> Result<Series, HistoryError> {
        let conn = lock(&self.reader);
        let mut series = Series {
            point: point.to_owned(),
            from,
            to,
            ..Series::default()
        };
        let Some(id) = conn
            .query_row("SELECT id FROM points WHERE point = ?1", [point], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?
        else {
            return Ok(series);
        };
        let (from_ts, to_ts) = (ts(from), ts(to));
        let row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<(u64, Value)> {
            Ok((
                ms(r.get(0)?),
                decode(&r.get::<_, String>(1)?, r.get(2)?, r.get(3)?),
            ))
        };
        series.before = conn
            .query_row(
                "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 1",
                params![id, from_ts],
                row,
            )
            .optional()?;
        let (total, numeric): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COUNT(num) FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3",
            params![id, from_ts, to_ts],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        series.total = u64::try_from(total).unwrap_or(0);
        let max = i64::try_from(max_points.max(1)).unwrap_or(i64::MAX);
        let edge = |order: &str| {
            conn.query_row(
                &format!(
                    "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3 ORDER BY ts {order} LIMIT 1"
                ),
                params![id, from_ts, to_ts],
                row,
            )
            .optional()
        };
        series.first = edge("ASC")?;
        series.last = edge("DESC")?;

        // Further back than the raw changes are kept: the hourly statistics.
        let pruned = mark(&conn, "pruned_before")?;
        if pruned.is_some_and(|p| from_ts < p)
            && let Some(buckets) = hourly_buckets(&conn, id, from_ts, to_ts, max)?
        {
            series.buckets = buckets;
            series.hourly = true;
            return Ok(series);
        }

        if total <= max {
            let mut stmt = conn.prepare(
                "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3 ORDER BY ts",
            )?;
            series.raw = stmt
                .query_map(params![id, from_ts, to_ts], row)?
                .collect::<Result<_, _>>()?;
        } else if numeric > 0 {
            // Numeric points are bucketed even if some readings were null
            // (device offline a moment): nulls simply do not count.
            let width = ((to_ts - from_ts) / max).max(1);
            let mut stmt = conn.prepare(
                "SELECT MIN(ts), AVG(num), MIN(num), MAX(num), COUNT(*) FROM samples
                 WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3 AND num IS NOT NULL
                 GROUP BY (ts - ?2) / ?4 ORDER BY 1",
            )?;
            series.buckets = stmt
                .query_map(params![id, from_ts, to_ts, width], |r| {
                    Ok(Bucket {
                        ts: ms(r.get(0)?),
                        avg: r.get(1)?,
                        min: r.get(2)?,
                        max: r.get(3)?,
                        count: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                    })
                })?
                .collect::<Result<_, _>>()?;
        } else {
            let mut stmt = conn.prepare(
                "SELECT ts, kind, num, txt FROM samples WHERE point_id = ?1 AND ts BETWEEN ?2 AND ?3
                 ORDER BY ts DESC LIMIT ?4",
            )?;
            let mut raw: Vec<_> = stmt
                .query_map(params![id, from_ts, to_ts, max], row)?
                .collect::<Result<_, _>>()?;
            raw.reverse();
            series.raw = raw;
            series.truncated = true;
        }
        Ok(series)
    }

    /// Runs an agent's query: one read-only statement, bounded in time and rows.
    pub fn query_sql(&self, sql: &str, max_rows: usize) -> Result<SqlResult, HistoryError> {
        let conn = lock(&self.agents);
        let started = Instant::now();
        conn.progress_handler(10_000, Some(move || started.elapsed() > SQL_BUDGET))?;
        let result = run_sql(&conn, sql, max_rows);
        conn.progress_handler(0, None::<fn() -> bool>)?;
        // Belt and braces: never leave a transaction (and its snapshot) open.
        if !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result.map_err(|e| match e {
            HistoryError::Db(rusqlite::Error::SqliteFailure(f, _))
                if f.code == rusqlite::ErrorCode::OperationInterrupted =>
            {
                HistoryError::TooSlow
            }
            other => other,
        })
    }
}

fn mark(conn: &Connection, name: &str) -> Result<Option<i64>, HistoryError> {
    Ok(conn
        .query_row("SELECT value FROM marks WHERE name = ?1", [name], |r| {
            r.get(0)
        })
        .optional()?)
}

fn set_mark(conn: &Connection, name: &str, value: i64) -> Result<(), HistoryError> {
    conn.execute(
        "INSERT INTO marks(name, value) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET value = excluded.value",
        params![name, value],
    )?;
    Ok(())
}

/// A window of a numeric point, `max` buckets at most, from its hourly
/// statistics (and, for the hours not compiled yet, its raw changes).
/// `None` when the point has none (text, or never a number).
fn hourly_buckets(
    conn: &Connection,
    id: i64,
    from: i64,
    to: i64,
    max: i64,
) -> Result<Option<Vec<Bucket>>, HistoryError> {
    let start = hourly::floor(from);
    let row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Hour> {
        Ok(Hour {
            start: r.get(0)?,
            mean: r.get(1)?,
            min: r.get(2)?,
            max: r.get(3)?,
            last: r.get(4)?,
        })
    };
    let mut hours: HashMap<i64, Hour> = conn
        .prepare(
            "SELECT hour, mean, min, max, last FROM hourly
             WHERE point_id = ?1 AND hour >= ?2 AND hour < ?3",
        )?
        .query_map(params![id, start, to], row)?
        .map(|h| h.map(|h| (h.start, h)))
        .collect::<Result<_, _>>()?;
    let mut held: Option<f64> = conn
        .query_row(
            "SELECT last FROM hourly WHERE point_id = ?1 AND hour < ?2 ORDER BY hour DESC LIMIT 1",
            params![id, start],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    // The hours not compiled yet (the last one or two): from the raw changes.
    let compiled = mark(conn, "hourly_until")?.unwrap_or(start).max(start);
    if compiled < to {
        let at_compiled: Option<f64> = conn
            .query_row(
                "SELECT num FROM samples WHERE point_id = ?1 AND ts < ?2 ORDER BY ts DESC LIMIT 1",
                params![id, compiled],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let changes: Vec<(i64, Option<f64>)> = conn
            .prepare("SELECT ts, num FROM samples WHERE point_id = ?1 AND ts >= ?2 AND ts < ?3 ORDER BY ts")?
            .query_map(params![id, compiled, to], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for h in hourly::hours(at_compiled, &changes, compiled, to + HOUR_MS, to) {
            hours.insert(h.start, h);
        }
    }
    if hours.is_empty() && held.is_none() {
        return Ok(None);
    }
    // Whole hours per bucket, as many as `max` allows.
    let count = (to - start + HOUR_MS - 1) / HOUR_MS;
    let width = ((count + max - 1) / max).max(1);
    let mut buckets = Vec::new();
    let mut hour = start;
    while hour < to {
        let mut bucket: Option<(f64, f64, f64, u64)> = None;
        for h in (0..width)
            .map(|i| hour + i * HOUR_MS)
            .take_while(|h| *h < to)
        {
            let this = hours
                .get(&h)
                .copied()
                .unwrap_or_else(|| Hour::held(h, held));
            held = this.last;
            let (Some(mean), Some(lo), Some(hi)) = (this.mean, this.min, this.max) else {
                continue;
            };
            let (sum, min, max, n) = bucket.unwrap_or((0.0, lo, hi, 0));
            bucket = Some((sum + mean, min.min(lo), max.max(hi), n + 1));
        }
        if let Some((sum, min, max, n)) = bucket {
            #[allow(clippy::cast_precision_loss)]
            buckets.push(Bucket {
                ts: ms(hour.max(from)),
                avg: sum / n as f64,
                min,
                max,
                count: n,
            });
        }
        hour += width * HOUR_MS;
    }
    Ok(Some(buckets))
}

fn run_sql(conn: &Connection, sql: &str, max_rows: usize) -> Result<SqlResult, HistoryError> {
    let mut stmt = conn.prepare(sql.trim().trim_end_matches(';'))?;
    if !stmt.readonly() {
        return Err(HistoryError::NotReadOnly);
    }
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
    let width = columns.len();
    let mut rows = Vec::new();
    let mut truncated = false;
    let mut bytes = 0usize;
    let mut cursor = stmt.query([])?;
    while let Some(row) = cursor.next()? {
        if rows.len() == max_rows || bytes > AGENT_MAX_ANSWER {
            truncated = true;
            break;
        }
        let mut out = Vec::with_capacity(width);
        for i in 0..width {
            out.push(match row.get_ref(i)? {
                ValueRef::Null => Json::Null,
                ValueRef::Integer(n) => n.into(),
                ValueRef::Real(f) => {
                    serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number)
                }
                ValueRef::Text(t) => {
                    bytes += t.len();
                    String::from_utf8_lossy(t).into_owned().into()
                }
                ValueRef::Blob(b) => format!("<{} bytes>", b.len()).into(),
            });
            bytes += 8;
        }
        rows.push(out);
    }
    Ok(SqlResult {
        columns,
        rows,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("moli-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("history.db");
        (Store::open(&path).unwrap(), dir)
    }

    fn p(s: &str) -> PointId {
        PointId::from(s)
    }

    #[test]
    fn values_round_trip_exactly() {
        let (store, dir) = store("types");
        let rows = vec![
            (p("d/on"), Value::Bool(true), 1_000),
            (p("d/on"), Value::Bool(false), 2_000),
            (p("d/n"), Value::Int(42), 1_000),
            (p("d/f"), Value::Float(21.5), 1_000),
            (p("d/t"), Value::from("short_release"), 1_000),
            (p("d/z"), Value::Null, 1_000),
        ];
        store.insert(&rows, false).unwrap();
        let s = store.series("d/on", 0, 10_000, 100).unwrap();
        assert_eq!(
            s.raw,
            vec![(1_000, Value::Bool(true)), (2_000, Value::Bool(false))]
        );
        assert_eq!(
            store.series("d/n", 0, 10_000, 100).unwrap().raw[0].1,
            Value::Int(42)
        );
        assert_eq!(
            store.series("d/t", 0, 10_000, 100).unwrap().raw[0].1,
            Value::from("short_release")
        );
        assert_eq!(
            store.series("d/z", 0, 10_000, 100).unwrap().raw[0].1,
            Value::Null
        );
        assert_eq!(store.series("unknown", 0, 10_000, 100).unwrap().total, 0);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_numeric_series_are_bucketed_with_a_starting_value() {
        let (store, dir) = store("buckets");
        let rows: Vec<_> = (0..1_000u32)
            .map(|i| {
                (
                    p("d/power"),
                    Value::Float(f64::from(i)),
                    10_000 + u64::from(i) * 10,
                )
            })
            .collect();
        store.insert(&rows, false).unwrap();
        store
            .insert(&[(p("d/power"), Value::Float(-1.0), 5_000)], false)
            .unwrap();
        let s = store.series("d/power", 10_000, 20_000, 100).unwrap();
        assert_eq!(s.total, 1_000);
        assert!(s.raw.is_empty());
        assert!(
            s.buckets.len() <= 101 && s.buckets.len() >= 99,
            "{}",
            s.buckets.len()
        );
        assert!(s.buckets[0].min.abs() < 1e-9);
        assert_eq!(s.before, Some((5_000, Value::Float(-1.0))));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn same_millisecond_changes_are_all_kept_and_restatements_are_not() {
        let (store, dir) = store("same-ms");
        let press = || (p("remote/button1"), Value::from("short_release"), 5_000);
        store.insert(&[press(), press()], false).unwrap();
        assert_eq!(
            store.series("remote/button1", 0, 10_000, 10).unwrap().total,
            2
        );
        // Re-recording the current state (after a restart or a lag) adds nothing.
        store.insert(&[press()], true).unwrap();
        assert_eq!(
            store.series("remote/button1", 0, 10_000, 10).unwrap().total,
            2
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn nulls_do_not_prevent_bucketing() {
        let (store, dir) = store("nulls");
        let mut rows: Vec<_> = (0..500u32)
            .map(|i| (p("d/t"), Value::Float(f64::from(i)), 1_000 + u64::from(i)))
            .collect();
        rows.push((p("d/t"), Value::Null, 1_250));
        store.insert(&rows, false).unwrap();
        let s = store.series("d/t", 0, 10_000, 50).unwrap();
        assert!(!s.buckets.is_empty());
        assert_eq!(s.first.map(|f| f.1), Some(Value::Float(0.0)));
        assert_eq!(s.last.map(|l| l.1), Some(Value::Float(499.0)));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn prune_removes_old_samples() {
        let (store, dir) = store("prune");
        let rows = [
            (p("d/x"), Value::Int(1), 1_000),
            (p("d/x"), Value::Int(2), 2_000),
            (p("d/x"), Value::Int(3), 9_000),
            (p("d/stable"), Value::Int(7), 1_000),
        ];
        store.insert(&rows, false).unwrap();
        assert_eq!(store.prune(5_000).unwrap(), 1);
        let x = store.series("d/x", 5_000, 10_000, 10).unwrap();
        assert_eq!(x.total, 1);
        // The value held when the window opens is still known…
        assert_eq!(x.before, Some((2_000, Value::Int(2))));
        // …and so is a value that never changed, however old.
        let stable = store.series("d/stable", 6_000, 10_000, 10).unwrap();
        assert_eq!(stable.before, Some((1_000, Value::Int(7))));
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    const HOUR: u64 = 3_600_000;
    /// Some day, far enough from the epoch.
    const DAY0: u64 = 20_000 * 24 * HOUR;

    /// Compiles until caught up; returns how many passes it took.
    fn compile_all(store: &Store, now: u64) -> usize {
        (1..=100)
            .find(|_| store.compile(now).unwrap().1)
            .expect("caught up")
    }

    #[test]
    fn hours_outlive_the_raw_changes() {
        let (store, dir) = store("hourly");
        // Hour k: the temperature is k, from 10 minutes past the hour (the
        // first 10 minutes it is still the previous hour's), for 3 days.
        let rows: Vec<_> = (0..72u32)
            .map(|k| {
                (
                    p("d/t"),
                    Value::Float(f64::from(k)),
                    DAY0 + u64::from(k) * HOUR + 600_000,
                )
            })
            .chain([(p("d/label"), Value::from("hello"), DAY0 + 5 * HOUR)])
            .collect();
        store.insert(&rows, false).unwrap();
        let now = DAY0 + 72 * HOUR + 10 * 60_000;
        assert_eq!(compile_all(&store, now), 3, "a day per pass");
        // Hour 5: 4 for 10 minutes, 5 for 50: weighted by time.
        let r = store
            .query_sql(
                &format!("SELECT mean, min, max, last FROM history_hourly WHERE point = 'd/t' AND hour = {}", DAY0 + 5 * HOUR),
                10,
            )
            .unwrap();
        let mean = r.rows[0][0].as_f64().unwrap();
        assert!(
            (mean - (4.0 * 10.0 + 5.0 * 50.0) / 60.0).abs() < 1e-9,
            "{mean}"
        );
        assert_eq!(
            r.rows[0][1..],
            [Json::from(4.0), Json::from(5.0), Json::from(5.0)]
        );
        // Text has no statistics.
        let text = store
            .query_sql(
                "SELECT COUNT(*) FROM history_hourly WHERE point = 'd/label'",
                1,
            )
            .unwrap();
        assert_eq!(text.rows[0][0], Json::from(0));

        // The first two days' raw changes go…
        assert!(store.prune(DAY0 + 48 * HOUR).unwrap() > 40);
        // …but a 3-day chart still has its curve, from the hours.
        let s = store.series("d/t", DAY0, DAY0 + 72 * HOUR, 36).unwrap();
        assert!(s.hourly);
        assert_eq!(s.buckets.len(), 36, "two hours a bucket");
        let second = &s.buckets[1];
        assert_eq!(second.ts, DAY0 + 2 * HOUR);
        assert_eq!((second.min, second.max, second.count), (1.0, 3.0, 2));
        // The last hours come from the raw changes still there.
        assert_eq!(s.buckets.last().unwrap().max, 71.0);
        // A window the raw changes still cover reads them, as before.
        let recent = store
            .series("d/t", DAY0 + 60 * HOUR, DAY0 + 72 * HOUR, 100)
            .unwrap();
        assert!(!recent.hourly);
        assert_eq!(recent.raw.len(), 12);
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pruning_never_outruns_the_statistics() {
        let (store, dir) = store("prune-hourly");
        let rows: Vec<_> = (0..10u32)
            .map(|k| {
                (
                    p("d/t"),
                    Value::Float(f64::from(k)),
                    DAY0 + u64::from(k) * HOUR,
                )
            })
            .collect();
        store.insert(&rows, false).unwrap();
        // Compiled up to hour 5 only (as if Moli had stopped then).
        assert!(store.compile(DAY0 + 5 * HOUR + 10 * 60_000).unwrap().1);
        store.prune(DAY0 + 100 * HOUR).unwrap();
        let left = store.query_sql("SELECT MIN(ts) FROM history", 1).unwrap();
        assert_eq!(
            left.rows[0][0],
            Json::from(DAY0 + 4 * HOUR),
            "hour 4 is held into hour 5"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agents_read_but_never_write() {
        let (store, dir) = store("sql");
        store
            .insert(
                &[
                    (p("door/contact"), Value::Bool(false), 1_000),
                    (p("door/contact"), Value::Bool(true), 2_000),
                ],
                false,
            )
            .unwrap();
        let r = store
            .query_sql(
                "SELECT point, COUNT(*) AS changes FROM history GROUP BY point;",
                10,
            )
            .unwrap();
        assert_eq!(r.columns, ["point", "changes"]);
        assert_eq!(
            r.rows,
            vec![vec![Json::from("door/contact"), Json::from(2)]]
        );

        for forbidden in [
            "DELETE FROM samples",
            "DROP VIEW history",
            "INSERT INTO points(point) VALUES ('x')",
            "ATTACH DATABASE '/etc/passwd' AS x",
            "PRAGMA query_only = 0",
            "SELECT 1; DELETE FROM samples",
            "BEGIN",
            "SAVEPOINT pin",
            "SELECT printf('%.*c', 100000000, 'x')",
            "SELECT zeroblob(100000000)",
        ] {
            assert!(
                store.query_sql(forbidden, 10).is_err(),
                "{forbidden} must be refused"
            );
        }
        assert_eq!(
            store.series("door/contact", 0, 10_000, 10).unwrap().total,
            2,
            "nothing changed"
        );

        let capped = store.query_sql("SELECT ts FROM history", 1).unwrap();
        assert!(capped.truncated);
        let slow = store.query_sql(
            "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT COUNT(*) FROM c",
            10,
        );
        assert!(matches!(slow, Err(HistoryError::TooSlow)), "{slow:?}");
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
