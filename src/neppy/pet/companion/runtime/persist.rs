//! Pause state persistence in `companion.db` (not observed data, so
//! `delete_all` leaves it alone). Pause survives a restart.

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::neppy::config::Config;
use crate::neppy::pet::companion::store;

const PAUSE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS companion_runtime_state (
  id INTEGER PRIMARY KEY CHECK (id = 1), paused INTEGER NOT NULL, paused_until TEXT
);";

pub fn load_pause(config: &Config) -> Result<(bool, Option<DateTime<Utc>>)> {
    store::with_connection(config, |c| {
        c.execute_batch(PAUSE_SCHEMA)?;
        let row: Option<(i64, Option<String>)> =
            rusqlite::OptionalExtension::optional(c.query_row(
                "SELECT paused, paused_until FROM companion_runtime_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ))?;
        Ok(match row {
            Some((p, until)) => (
                p != 0,
                until
                    .and_then(|u| DateTime::parse_from_rfc3339(&u).ok())
                    .map(|d| d.with_timezone(&Utc)),
            ),
            None => (false, None),
        })
    })
}

pub fn save_pause(config: &Config, paused: bool, until: Option<DateTime<Utc>>) -> Result<()> {
    store::with_connection(config, |c| {
        c.execute_batch(PAUSE_SCHEMA)?;
        c.execute(
            "INSERT INTO companion_runtime_state (id, paused, paused_until) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET paused = excluded.paused,
               paused_until = excluded.paused_until",
            rusqlite::params![paused as i64, until.map(|u| u.to_rfc3339())],
        )?;
        Ok(())
    })
}
