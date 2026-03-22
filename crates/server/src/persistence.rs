use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::decider::DecidedEvent;

#[derive(Clone, Debug)]
pub(crate) struct ProviderSessionBinding {
    pub thread_id: String,
    pub provider_name: String,
    pub runtime_mode: String,
    pub provider_session_id: Option<String>,
    pub status: String,
    pub active_turn_id: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Clone)]
pub(crate) struct Persistence {
    connection: Arc<Mutex<Connection>>,
}

impl Persistence {
    pub(crate) fn new(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir).with_context(|| {
            format!(
                "failed to create server state directory at {}",
                state_dir.to_string_lossy()
            )
        })?;
        let database_path = state_dir.join("server.sqlite3");
        let connection = Connection::open(&database_path).with_context(|| {
            format!(
                "failed to open sqlite database at {}",
                database_path.to_string_lossy()
            )
        })?;
        connection.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS orchestration_events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL,
                aggregate_kind TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                command_id TEXT,
                causation_event_id TEXT,
                correlation_id TEXT,
                metadata_json TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orchestration_command_receipts (
                command_id TEXT PRIMARY KEY,
                sequence INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS provider_session_runtime (
                thread_id TEXT PRIMARY KEY,
                provider_name TEXT NOT NULL,
                runtime_mode TEXT NOT NULL,
                provider_session_id TEXT,
                status TEXT NOT NULL,
                active_turn_id TEXT,
                last_error TEXT,
                updated_at TEXT NOT NULL
            );
            "#,
        )?;

        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub(crate) async fn load_events(&self) -> Result<Vec<Value>> {
        let connection = self.connection.lock().await;
        let mut statement = connection.prepare(
            r#"
            SELECT
                sequence,
                event_id,
                aggregate_kind,
                aggregate_id,
                occurred_at,
                command_id,
                causation_event_id,
                correlation_id,
                metadata_json,
                event_type,
                payload_json
            FROM orchestration_events
            ORDER BY sequence ASC
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            let metadata_json: String = row.get(8)?;
            let payload_json: String = row.get(10)?;
            let metadata = serde_json::from_str::<Value>(&metadata_json).unwrap_or_else(|_| json!({}));
            let payload = serde_json::from_str::<Value>(&payload_json).unwrap_or(Value::Null);
            Ok(json!({
                "sequence": row.get::<_, i64>(0)?,
                "eventId": row.get::<_, String>(1)?,
                "aggregateKind": row.get::<_, String>(2)?,
                "aggregateId": row.get::<_, String>(3)?,
                "occurredAt": row.get::<_, String>(4)?,
                "commandId": row.get::<_, Option<String>>(5)?,
                "causationEventId": row.get::<_, Option<String>>(6)?,
                "correlationId": row.get::<_, Option<String>>(7)?,
                "metadata": metadata,
                "type": row.get::<_, String>(9)?,
                "payload": payload,
            }))
        })?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row?);
        }
        Ok(events)
    }

    pub(crate) async fn find_command_receipt(&self, command_id: &str) -> Result<Option<u64>> {
        let connection = self.connection.lock().await;
        let sequence = connection
            .query_row(
                "SELECT sequence FROM orchestration_command_receipts WHERE command_id = ?1",
                [command_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(sequence.map(|value| value as u64))
    }

    pub(crate) async fn append_events(
        &self,
        decided_events: &[DecidedEvent],
        make_event_id: impl Fn() -> String,
    ) -> Result<Vec<Value>> {
        let mut connection = self.connection.lock().await;
        let transaction = connection.transaction()?;
        let mut appended = Vec::with_capacity(decided_events.len());
        let mut last_command_receipt: Option<(String, u64)> = None;

        for decided_event in decided_events {
            let metadata_json = serde_json::to_string(&decided_event.metadata)?;
            let payload_json = serde_json::to_string(&decided_event.payload)?;
            let event_id = make_event_id();
            transaction.execute(
                r#"
                INSERT INTO orchestration_events (
                    event_id,
                    aggregate_kind,
                    aggregate_id,
                    occurred_at,
                    command_id,
                    causation_event_id,
                    correlation_id,
                    metadata_json,
                    event_type,
                    payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
                params![
                    event_id,
                    decided_event.aggregate_kind,
                    decided_event.aggregate_id,
                    decided_event.occurred_at,
                    decided_event.command_id,
                    decided_event.causation_event_id,
                    decided_event.correlation_id,
                    metadata_json,
                    decided_event.event_type,
                    payload_json,
                ],
            )?;
            let sequence = transaction.last_insert_rowid() as u64;
            if let Some(command_id) = decided_event.command_id.clone() {
                last_command_receipt = Some((command_id, sequence));
            }
            appended.push(json!({
                "sequence": sequence,
                "eventId": event_id,
                "aggregateKind": decided_event.aggregate_kind,
                "aggregateId": decided_event.aggregate_id,
                "occurredAt": decided_event.occurred_at,
                "commandId": decided_event.command_id,
                "causationEventId": decided_event.causation_event_id,
                "correlationId": decided_event.correlation_id,
                "metadata": decided_event.metadata,
                "type": decided_event.event_type,
                "payload": decided_event.payload,
            }));
        }

        if let Some((command_id, sequence)) = last_command_receipt {
            transaction.execute(
                r#"
                INSERT INTO orchestration_command_receipts (command_id, sequence)
                VALUES (?1, ?2)
                ON CONFLICT(command_id) DO UPDATE SET sequence = excluded.sequence
                "#,
                params![command_id, sequence as i64],
            )?;
        }

        transaction.commit()?;
        Ok(appended)
    }

    pub(crate) async fn load_provider_session_bindings(&self) -> Result<Vec<ProviderSessionBinding>> {
        let connection = self.connection.lock().await;
        let mut statement = connection.prepare(
            r#"
            SELECT
                thread_id,
                provider_name,
                runtime_mode,
                provider_session_id,
                status,
                active_turn_id,
                last_error,
                updated_at
            FROM provider_session_runtime
            ORDER BY updated_at ASC
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProviderSessionBinding {
                thread_id: row.get(0)?,
                provider_name: row.get(1)?,
                runtime_mode: row.get(2)?,
                provider_session_id: row.get(3)?,
                status: row.get(4)?,
                active_turn_id: row.get(5)?,
                last_error: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })?;

        let mut bindings = Vec::new();
        for row in rows {
            bindings.push(row?);
        }
        Ok(bindings)
    }

    pub(crate) async fn upsert_provider_session_binding(
        &self,
        binding: &ProviderSessionBinding,
    ) -> Result<()> {
        let connection = self.connection.lock().await;
        connection.execute(
            r#"
            INSERT INTO provider_session_runtime (
                thread_id,
                provider_name,
                runtime_mode,
                provider_session_id,
                status,
                active_turn_id,
                last_error,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(thread_id) DO UPDATE SET
                provider_name = excluded.provider_name,
                runtime_mode = excluded.runtime_mode,
                provider_session_id = excluded.provider_session_id,
                status = excluded.status,
                active_turn_id = excluded.active_turn_id,
                last_error = excluded.last_error,
                updated_at = excluded.updated_at
            "#,
            params![
                binding.thread_id,
                binding.provider_name,
                binding.runtime_mode,
                binding.provider_session_id,
                binding.status,
                binding.active_turn_id,
                binding.last_error,
                binding.updated_at,
            ],
        )?;
        Ok(())
    }

    pub(crate) async fn delete_provider_session_binding(&self, thread_id: &str) -> Result<()> {
        let connection = self.connection.lock().await;
        connection.execute(
            "DELETE FROM provider_session_runtime WHERE thread_id = ?1",
            [thread_id],
        )?;
        Ok(())
    }
}
