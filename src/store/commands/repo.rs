//! Synchronous SQL for the commands. Every write is one `BEGIN IMMEDIATE`
//! transaction.

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection, Row, TransactionBehavior};

use super::{Accepted, Command, CommandError, CommandState, NewCommand, Outcome, MAX_ATTEMPTS};
use crate::store::history::Millis;

type Result<T> = std::result::Result<T, CommandError>;

const COLUMNS: &str = "id, kind, payload, subject, state, attempts, created_at, updated_at, \
     finished_at, outcome";

/// Reason recorded for a command given up after [`MAX_ATTEMPTS`] starts.
const GIVEN_UP: &str = "worker가 이 명령을 처리하다 여러 번 멈춰서 더는 시도하지 않아요.";

fn command_from_row(row: &Row<'_>) -> Result<Command> {
    let id: String = row.get(0)?;
    let state: String = row.get(4)?;
    let state = state.parse().map_err(|()| {
        CommandError::from(rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown command state {state:?}").into(),
        ))
    })?;
    let outcome: Option<String> = row.get(9)?;
    let outcome = outcome
        .map(|text| serde_json::from_str::<Outcome>(&text))
        .transpose()
        .map_err(|_| CommandError::Outcome(id.clone()))?;
    Ok(Command {
        id,
        kind: row.get(1)?,
        payload: row.get(2)?,
        subject: row.get(3)?,
        state,
        attempts: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        finished_at: row.get(8)?,
        outcome,
    })
}

fn select_one(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<Option<Command>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(args)?;
    match rows.next()? {
        Some(row) => Ok(Some(command_from_row(row)?)),
        None => Ok(None),
    }
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Command>> {
    select_one(
        conn,
        &format!("SELECT {COLUMNS} FROM commands WHERE id = ?1"),
        [id],
    )
}

pub fn accept(conn: &mut Connection, new: &NewCommand, now: Millis) -> Result<Accepted> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    if let Some(existing) = get(&tx, &new.id)? {
        return Ok(
            if existing.kind == new.kind && existing.payload == new.payload {
                Accepted::Existing(existing)
            } else {
                Accepted::Mismatch(existing)
            },
        );
    }

    if let Some(subject) = &new.subject {
        let open = select_one(
            &tx,
            &format!(
                "SELECT {COLUMNS} FROM commands
                 WHERE kind = ?1 AND subject = ?2 AND state IN ('pending', 'running')
                 ORDER BY seq LIMIT 1"
            ),
            params![new.kind, subject],
        )?;
        if let Some(open) = open {
            return Ok(Accepted::Busy(open));
        }
    }

    tx.execute(
        "INSERT INTO commands (id, kind, payload, subject, state, attempts, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?5)",
        params![new.id, new.kind, new.payload, new.subject, now],
    )?;
    let created = get(&tx, &new.id)?.expect("the command was just inserted");
    tx.commit()?;
    Ok(Accepted::Created(created))
}

pub fn open_for_subjects(
    conn: &Connection,
    kind: &str,
    subjects: &[String],
) -> Result<HashMap<String, Command>> {
    if subjects.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; subjects.len()].join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM commands
         WHERE kind = ? AND state IN ('pending', 'running') AND subject IN ({placeholders})
         ORDER BY seq"
    ))?;
    let args = std::iter::once(kind).chain(subjects.iter().map(String::as_str));
    let mut rows = stmt.query(params_from_iter(args))?;
    let mut out = HashMap::new();
    while let Some(row) = rows.next()? {
        let command = command_from_row(row)?;
        if let Some(subject) = command.subject.clone() {
            // The oldest open command of a subject is the one being worked on.
            out.entry(subject).or_insert(command);
        }
    }
    Ok(out)
}

pub fn has_open(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM commands WHERE state IN ('pending', 'running'))",
        [],
        |row| row.get(0),
    )?)
}

pub fn running_count(conn: &Connection) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM commands WHERE state = 'running'",
        [],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

pub fn claim_next(conn: &mut Connection, now: Millis) -> Result<Option<Command>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    loop {
        let Some(command) = select_one(
            &tx,
            &format!(
                "SELECT {COLUMNS} FROM commands WHERE state IN ('pending', 'running')
                 ORDER BY seq LIMIT 1"
            ),
            [],
        )?
        else {
            tx.commit()?;
            return Ok(None);
        };

        if command.attempts >= MAX_ATTEMPTS {
            let outcome = Outcome {
                result: "failed".to_owned(),
                reason: Some(GIVEN_UP.to_owned()),
            };
            end(&tx, &command.id, CommandState::Failed, &outcome, now)?;
            continue;
        }

        tx.execute(
            "UPDATE commands SET state = 'running', attempts = attempts + 1, updated_at = ?2
             WHERE id = ?1",
            params![command.id, now],
        )?;
        let claimed = get(&tx, &command.id)?.expect("the command was just updated");
        tx.commit()?;
        return Ok(Some(claimed));
    }
}

fn end(
    conn: &Connection,
    id: &str,
    state: CommandState,
    outcome: &Outcome,
    now: Millis,
) -> Result<bool> {
    let outcome = serde_json::to_string(outcome).expect("an outcome serializes");
    let changed = conn.execute(
        "UPDATE commands
         SET state = ?2, outcome = ?3, updated_at = ?4, finished_at = ?4
         WHERE id = ?1 AND state IN ('pending', 'running')",
        params![id, state.code(), outcome, now],
    )?;
    Ok(changed == 1)
}

pub fn finish(
    conn: &mut Connection,
    id: &str,
    state: CommandState,
    outcome: &Outcome,
    now: Millis,
) -> Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ended = end(&tx, id, state, outcome, now)?;
    tx.commit()?;
    Ok(ended)
}
