use std::path::Path;
use std::sync::{Arc, Mutex};

use ayu_core::{
    AccountId, AuthState, AuthStateRepository, AyuAuditRepository, AyuDeletedMessage,
    AyuEditedRevision, CompactDialog, CompactMessage, CoreError, DialogFlags, DialogId,
    DialogRepository, MediaDescriptor, MediaKind, MessageEntitySpan, MessageFlags,
    MessageRepository, MsgId, MtpAuthorizationBlob, PeerId, RegexFilterGlobalExclusion,
    RegexFilterRule, SyncStateRepository, SyncStateSnapshot, TopicId, UserId,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use smallvec::SmallVec;

use crate::error::StorageError;

/// SQLite storage backend configured for WAL mode and bounded RAM footprint.
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    /// Opens (or creates) a disk-backed SQLite database in WAL mode with capped 4 MB page cache.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        Self::configure_connection(&conn)?;
        Self::initialize_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Opens an ephemeral in-memory SQLite database (primarily for fast unit tests).
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_connection(&conn)?;
        Self::initialize_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Returns the active SQLite `journal_mode` pragma (e.g. `"wal"` for disk DBs).
    pub fn journal_mode(&self) -> Result<String, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mode: String = guard.query_row("PRAGMA journal_mode;", [], |row| row.get(0))?;
        Ok(mode)
    }

    fn configure_connection(conn: &Connection) -> Result<(), StorageError> {
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA temp_store = MEMORY;
            PRAGMA cache_size = -1024;
            PRAGMA mmap_size = 0;
            PRAGMA foreign_keys = ON;
            ",
        )?;
        Ok(())
    }

    fn initialize_schema(conn: &Connection) -> Result<(), StorageError> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS messages (
                account_id INTEGER NOT NULL,
                dialog_id INTEGER NOT NULL,
                msg_id INTEGER NOT NULL,
                sender_id INTEGER NOT NULL,
                grouped_id INTEGER NOT NULL,
                reply_to_msg_id INTEGER NOT NULL,
                date_unix INTEGER NOT NULL,
                edit_date_unix INTEGER NOT NULL,
                views INTEGER NOT NULL,
                flags INTEGER NOT NULL,
                author_name TEXT,
                text TEXT NOT NULL,
                entities BLOB NOT NULL,
                media_kind INTEGER NOT NULL,
                media_file_id INTEGER,
                media_access_hash INTEGER,
                media_size_bytes INTEGER,
                media_mime_type TEXT,
                media_local_path TEXT,
                media_thumb_path TEXT,
                media_width INTEGER,
                media_height INTEGER,
                media_duration_secs INTEGER,
                media_has_spoiler INTEGER,
                PRIMARY KEY (account_id, dialog_id, msg_id)
            );
            CREATE INDEX IF NOT EXISTS idx_messages_dialog_date
                ON messages (account_id, dialog_id, date_unix);

            CREATE TABLE IF NOT EXISTS dialogs (
                account_id INTEGER NOT NULL,
                dialog_id INTEGER NOT NULL,
                peer_id INTEGER NOT NULL,
                folder_id INTEGER NOT NULL,
                title TEXT NOT NULL,
                draft_text TEXT,
                top_message_id INTEGER NOT NULL,
                read_inbox_max_id INTEGER NOT NULL,
                read_outbox_max_id INTEGER NOT NULL,
                last_message_date INTEGER NOT NULL,
                unread_count INTEGER NOT NULL,
                unread_mentions_count INTEGER NOT NULL,
                flags INTEGER NOT NULL,
                PRIMARY KEY (account_id, dialog_id)
            );
            CREATE INDEX IF NOT EXISTS idx_dialogs_folder_sort
                ON dialogs (account_id, folder_id, last_message_date DESC);

            CREATE TABLE IF NOT EXISTS ayu_deleted_messages (
                fake_id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                dialog_id INTEGER NOT NULL,
                topic_id INTEGER NOT NULL,
                msg_id INTEGER NOT NULL,
                sender_id INTEGER NOT NULL,
                grouped_id INTEGER NOT NULL,
                reply_to_msg_id INTEGER NOT NULL,
                date_unix INTEGER NOT NULL,
                edit_date_unix INTEGER NOT NULL,
                views INTEGER NOT NULL,
                flags INTEGER NOT NULL,
                entity_create_date INTEGER NOT NULL,
                author_name TEXT,
                text TEXT NOT NULL,
                entities BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ayu_deleted_lookup
                ON ayu_deleted_messages (account_id, dialog_id, topic_id, msg_id);

            CREATE TABLE IF NOT EXISTS ayu_edited_messages (
                fake_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                dialog_id INTEGER NOT NULL,
                msg_id INTEGER NOT NULL,
                sender_id INTEGER NOT NULL,
                grouped_id INTEGER NOT NULL,
                reply_to_msg_id INTEGER NOT NULL,
                date_unix INTEGER NOT NULL,
                edit_date_unix INTEGER NOT NULL,
                views INTEGER NOT NULL,
                flags INTEGER NOT NULL,
                entity_create_date INTEGER NOT NULL,
                author_name TEXT,
                text TEXT NOT NULL,
                entities BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_ayu_edited_lookup
                ON ayu_edited_messages (user_id, dialog_id, msg_id);

            CREATE TABLE IF NOT EXISTS ayu_regex_filters (
                id BLOB PRIMARY KEY,
                pattern TEXT NOT NULL,
                dialog_id INTEGER,
                enabled INTEGER NOT NULL,
                reversed INTEGER NOT NULL,
                case_insensitive INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS ayu_regex_exclusions (
                dialog_id INTEGER NOT NULL,
                filter_id BLOB NOT NULL,
                PRIMARY KEY (dialog_id, filter_id)
            );

            CREATE TABLE IF NOT EXISTS account_auth (
                account_id INTEGER PRIMARY KEY,
                account_index INTEGER NOT NULL,
                user_id INTEGER NOT NULL,
                main_dc_id INTEGER NOT NULL,
                is_test_dc INTEGER NOT NULL,
                is_premium INTEGER NOT NULL,
                auth_state INTEGER NOT NULL,
                serialized_blob BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS account_sync_state (
                account_id INTEGER PRIMARY KEY,
                pts INTEGER NOT NULL,
                qts INTEGER NOT NULL,
                date_unix INTEGER NOT NULL,
                seq INTEGER NOT NULL
            ) STRICT;

            CREATE TABLE IF NOT EXISTS channel_sync_pts (
                account_id INTEGER NOT NULL,
                channel_id INTEGER NOT NULL,
                pts INTEGER NOT NULL,
                PRIMARY KEY (account_id, channel_id)
            ) STRICT;

            CREATE TABLE IF NOT EXISTS ayu_deleted_records (
                fake_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                dialog_id INTEGER NOT NULL,
                topic_id INTEGER NOT NULL,
                msg_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                payload BLOB NOT NULL
            ) STRICT;
            CREATE INDEX IF NOT EXISTS idx_ayu_deleted_records_lookup
                ON ayu_deleted_records (user_id, dialog_id, topic_id, msg_id);

            CREATE TABLE IF NOT EXISTS ayu_edited_records (
                fake_id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                dialog_id INTEGER NOT NULL,
                msg_id INTEGER NOT NULL,
                payload BLOB NOT NULL
            ) STRICT;
            CREATE INDEX IF NOT EXISTS idx_ayu_edited_records_lookup
                ON ayu_edited_records (user_id, dialog_id, msg_id);

            CREATE TABLE IF NOT EXISTS kv_cache (
                ns TEXT NOT NULL,
                key_hi INTEGER NOT NULL,
                key_lo INTEGER NOT NULL,
                tag INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                last_used INTEGER NOT NULL,
                data BLOB NOT NULL,
                PRIMARY KEY (ns, key_hi, key_lo)
            ) STRICT;
            CREATE INDEX IF NOT EXISTS idx_kv_cache_ns_lru
                ON kv_cache (ns, last_used ASC);
            CREATE INDEX IF NOT EXISTS idx_kv_cache_ns_tag
                ON kv_cache (ns, tag);
            ",
        )?;
        Ok(())
    }

    pub fn insert_deleted_record(
        &self,
        user_id: i64,
        dialog_id: i64,
        topic_id: i64,
        msg_id: i32,
        text: &str,
        payload: &[u8],
    ) -> Result<i64, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "INSERT INTO ayu_deleted_records (user_id, dialog_id, topic_id, msg_id, text, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        stmt.execute(params![user_id, dialog_id, topic_id, msg_id, text, payload])?;
        Ok(guard.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn query_deleted_records(
        &self,
        user_id: i64,
        dialog_id: i64,
        topic_id: i64,
        min_id: i64,
        max_id: i64,
        limit: usize,
        search_query: &str,
    ) -> Result<Vec<(i64, Vec<u8>)>, StorageError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let like_pattern = (!search_query.is_empty()).then(|| Self::escape_like_pattern(search_query));
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "
            SELECT fake_id, payload
            FROM ayu_deleted_records
            WHERE user_id = ?1
              AND dialog_id = ?2
              AND (topic_id = ?3 OR ?3 = 0)
              AND (msg_id > ?4 OR ?4 = 0)
              AND (msg_id < ?5 OR ?5 = 0)
              AND (?6 IS NULL OR text LIKE ?6 ESCAPE '\\')
            ORDER BY msg_id DESC
            LIMIT ?7
            ",
        )?;
        let rows = stmt.query_map(
            params![user_id, dialog_id, topic_id, min_id, max_id, like_pattern, limit as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        let mut out = Vec::with_capacity(limit.min(64));
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn has_deleted_records(
        &self,
        user_id: i64,
        dialog_id: i64,
        topic_id: i64,
    ) -> Result<bool, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "SELECT 1 FROM ayu_deleted_records
             WHERE user_id = ?1 AND dialog_id = ?2 AND (topic_id = ?3 OR ?3 = 0)
             LIMIT 1",
        )?;
        Ok(stmt.query_row(params![user_id, dialog_id, topic_id], |_| Ok(())).optional()?.is_some())
    }

    pub fn remove_deleted_record(
        &self,
        user_id: i64,
        dialog_id: i64,
        msg_id: i32,
    ) -> Result<usize, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "DELETE FROM ayu_deleted_records WHERE user_id = ?1 AND dialog_id = ?2 AND msg_id = ?3",
        )?;
        Ok(stmt.execute(params![user_id, dialog_id, msg_id])?)
    }

    pub fn clear_deleted_records(
        &self,
        user_id: i64,
        dialog_id: i64,
        topic_id: i64,
    ) -> Result<usize, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "DELETE FROM ayu_deleted_records
             WHERE user_id = ?1 AND dialog_id = ?2 AND (topic_id = ?3 OR ?3 = 0)",
        )?;
        Ok(stmt.execute(params![user_id, dialog_id, topic_id])?)
    }

    pub fn insert_edited_record(
        &self,
        user_id: i64,
        dialog_id: i64,
        msg_id: i32,
        payload: &[u8],
    ) -> Result<i64, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "INSERT INTO ayu_edited_records (user_id, dialog_id, msg_id, payload)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        stmt.execute(params![user_id, dialog_id, msg_id, payload])?;
        Ok(guard.last_insert_rowid())
    }

    pub fn query_edited_records(
        &self,
        user_id: i64,
        dialog_id: i64,
        msg_id: i32,
        min_id: i64,
        max_id: i64,
        limit: usize,
    ) -> Result<Vec<(i64, Vec<u8>)>, StorageError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "
            SELECT fake_id, payload
            FROM ayu_edited_records
            WHERE user_id = ?1
              AND dialog_id = ?2
              AND msg_id = ?3
              AND (fake_id > ?4 OR ?4 = 0)
              AND (fake_id < ?5 OR ?5 = 0)
            ORDER BY fake_id DESC
            LIMIT ?6
            ",
        )?;
        let rows = stmt.query_map(
            params![user_id, dialog_id, msg_id, min_id, max_id, limit as i64],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        let mut out = Vec::with_capacity(limit.min(64));
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn has_edited_records(
        &self,
        user_id: i64,
        dialog_id: i64,
        msg_id: i32,
    ) -> Result<bool, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "SELECT 1 FROM ayu_edited_records
             WHERE user_id = ?1 AND dialog_id = ?2 AND msg_id = ?3
             LIMIT 1",
        )?;
        Ok(stmt.query_row(params![user_id, dialog_id, msg_id], |_| Ok(())).optional()?.is_some())
    }

    fn kv_now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn kv_put(
        &self,
        ns: &str,
        key_hi: u64,
        key_lo: u64,
        tag: u8,
        data: &[u8],
        only_if_empty: bool,
        max_bytes: i64,
    ) -> Result<(), StorageError> {
        let now = Self::kv_now_secs();
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let sql = if only_if_empty {
            "INSERT OR IGNORE INTO kv_cache (ns, key_hi, key_lo, tag, size_bytes, last_used, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        } else {
            "INSERT OR REPLACE INTO kv_cache (ns, key_hi, key_lo, tag, size_bytes, last_used, data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
        };
        let mut stmt = guard.prepare_cached(sql)?;
        stmt.execute(params![
            ns,
            key_hi as i64,
            key_lo as i64,
            i64::from(tag),
            data.len() as i64,
            now,
            data,
        ])?;
        if max_bytes > 0 {
            let total: i64 = guard.query_row(
                "SELECT COALESCE(SUM(size_bytes), 0) FROM kv_cache WHERE ns = ?1",
                params![ns],
                |row| row.get(0),
            )?;
            if total > max_bytes {
                let mut excess = total - max_bytes;
                let mut select = guard.prepare_cached(
                    "SELECT key_hi, key_lo, size_bytes FROM kv_cache
                     WHERE ns = ?1 ORDER BY last_used ASC LIMIT 64",
                )?;
                let victims: Vec<(i64, i64, i64)> = select
                    .query_map(params![ns], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut del = guard.prepare_cached(
                    "DELETE FROM kv_cache WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
                )?;
                for (hi, lo, sz) in victims {
                    if excess <= 0 {
                        break;
                    }
                    del.execute(params![ns, hi, lo])?;
                    excess -= sz;
                }
            }
        }
        Ok(())
    }

    pub fn kv_get(
        &self,
        ns: &str,
        key_hi: u64,
        key_lo: u64,
    ) -> Result<Option<(u8, Vec<u8>)>, StorageError> {
        let now = Self::kv_now_secs();
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "SELECT tag, data FROM kv_cache WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
        )?;
        let found = stmt
            .query_row(params![ns, key_hi as i64, key_lo as i64], |row| {
                Ok((row.get::<_, i64>(0)? as u8, row.get::<_, Vec<u8>>(1)?))
            })
            .optional()?;
        if found.is_some() {
            let mut touch = guard.prepare_cached(
                "UPDATE kv_cache SET last_used = ?4 WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
            )?;
            let _ = touch.execute(params![ns, key_hi as i64, key_lo as i64, now]);
        }
        Ok(found)
    }

    pub fn kv_remove(&self, ns: &str, key_hi: u64, key_lo: u64) -> Result<(), StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "DELETE FROM kv_cache WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
        )?;
        stmt.execute(params![ns, key_hi as i64, key_lo as i64])?;
        Ok(())
    }

    pub fn kv_copy_or_move_if_empty(
        &self,
        ns: &str,
        from_hi: u64,
        from_lo: u64,
        to_hi: u64,
        to_lo: u64,
        is_move: bool,
    ) -> Result<(), StorageError> {
        let now = Self::kv_now_secs();
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let inserted = guard.execute(
            "INSERT OR IGNORE INTO kv_cache (ns, key_hi, key_lo, tag, size_bytes, last_used, data)
             SELECT ns, ?4, ?5, tag, size_bytes, ?6, data
             FROM kv_cache
             WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
            params![ns, from_hi as i64, from_lo as i64, to_hi as i64, to_lo as i64, now],
        )?;
        if is_move && inserted > 0 {
            guard.execute(
                "DELETE FROM kv_cache WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
                params![ns, from_hi as i64, from_lo as i64],
            )?;
        }
        Ok(())
    }

    pub fn kv_get_sizes(
        &self,
        ns: &str,
        keys: &[(u64, u64)],
    ) -> Result<Vec<i32>, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "SELECT size_bytes FROM kv_cache WHERE ns = ?1 AND key_hi = ?2 AND key_lo = ?3",
        )?;
        let mut out = Vec::with_capacity(keys.len());
        for &(hi, lo) in keys {
            let sz = stmt
                .query_row(params![ns, hi as i64, lo as i64], |row| row.get::<_, i64>(0))
                .optional()?
                .map(|v| v.clamp(0, i64::from(i32::MAX)) as i32)
                .unwrap_or(-1);
            out.push(sz);
        }
        Ok(out)
    }

    pub fn kv_clear(&self, ns: &str) -> Result<(), StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached("DELETE FROM kv_cache WHERE ns = ?1")?;
        stmt.execute(params![ns])?;
        Ok(())
    }

    pub fn kv_clear_by_tag(&self, ns: &str, tag: u8) -> Result<(), StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt =
            guard.prepare_cached("DELETE FROM kv_cache WHERE ns = ?1 AND tag = ?2")?;
        stmt.execute(params![ns, i64::from(tag)])?;
        Ok(())
    }

    pub fn kv_stats(&self, ns: &str) -> Result<Vec<(u8, i64, i64)>, StorageError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard.prepare_cached(
            "SELECT tag, COUNT(*), COALESCE(SUM(size_bytes), 0)
             FROM kv_cache WHERE ns = ?1 GROUP BY tag",
        )?;
        let rows = stmt.query_map(params![ns], |row| {
            Ok((row.get::<_, i64>(0)? as u8, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn decode_entities(blob: &[u8]) -> Result<SmallVec<[MessageEntitySpan; 2]>, StorageError> {
        let elem_size = std::mem::size_of::<MessageEntitySpan>();
        if blob.is_empty() {
            return Ok(SmallVec::new());
        }
        if !blob.len().is_multiple_of(elem_size) {
            return Err(StorageError::CorruptEntityBlob(blob.len()));
        }
        let mut out = SmallVec::with_capacity(blob.len() / elem_size);
        for chunk in blob.chunks_exact(elem_size) {
            if let Ok(span) = bytemuck::try_from_bytes::<MessageEntitySpan>(chunk) {
                out.push(*span);
            } else {
                let mut aligned = MessageEntitySpan::default();
                bytemuck::bytes_of_mut(&mut aligned).copy_from_slice(chunk);
                out.push(aligned);
            }
        }
        Ok(out)
    }

    fn decode_message_row(row: &Row<'_>) -> Result<CompactMessage, rusqlite::Error> {
        let dialog_id = DialogId(row.get(0)?);
        let msg_id = MsgId(row.get(1)?);
        let sender_raw: i64 = row.get(2)?;
        let grouped_raw: i64 = row.get(3)?;
        let reply_to_msg_id = MsgId(row.get(4)?);
        let date_unix: i32 = row.get(5)?;
        let edit_date_unix: i32 = row.get(6)?;
        let views_raw: i64 = row.get(7)?;
        let flags_raw: i64 = row.get(8)?;
        let author_name: Option<String> = row.get(9)?;
        let text: String = row.get(10)?;
        let entities_blob: Vec<u8> = row.get(11)?;
        let media_kind_raw: i64 = row.get(12)?;

        let media = if media_kind_raw > 0 {
            let file_id: i64 = row.get::<_, Option<i64>>(13)?.unwrap_or(0);
            let access_hash: i64 = row.get::<_, Option<i64>>(14)?.unwrap_or(0);
            let size_bytes: i64 = row.get::<_, Option<i64>>(15)?.unwrap_or(0);
            let mime_type: String = row.get::<_, Option<String>>(16)?.unwrap_or_default();
            let local_path: Option<String> = row.get(17)?;
            let thumb_path: Option<String> = row.get(18)?;
            let width: i64 = row.get::<_, Option<i64>>(19)?.unwrap_or(0);
            let height: i64 = row.get::<_, Option<i64>>(20)?.unwrap_or(0);
            let duration_secs: i64 = row.get::<_, Option<i64>>(21)?.unwrap_or(0);
            let has_spoiler: i64 = row.get::<_, Option<i64>>(22)?.unwrap_or(0);
            Some(Box::new(MediaDescriptor {
                file_id: file_id as u64,
                access_hash: access_hash as u64,
                size_bytes: size_bytes.max(0) as u64,
                mime_type: mime_type.into_boxed_str(),
                local_path: local_path.map(String::into_boxed_str),
                thumb_path: thumb_path.map(String::into_boxed_str),
                width: width.clamp(0, i64::from(u32::MAX)) as u32,
                height: height.clamp(0, i64::from(u32::MAX)) as u32,
                duration_secs: duration_secs.clamp(0, i64::from(u32::MAX)) as u32,
                kind: MediaKind::from_u8(media_kind_raw.clamp(0, 255) as u8),
                has_spoiler: has_spoiler != 0,
            }))
        } else {
            None
        };

        let entities = Self::decode_entities(&entities_blob).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Blob,
                Box::new(err),
            )
        })?;

        Ok(CompactMessage {
            dialog_id,
            sender_id: PeerId(sender_raw as u64),
            grouped_id: grouped_raw as u64,
            text: text.into_boxed_str(),
            author_name: author_name.map(|s| Arc::from(s.into_boxed_str())),
            media,
            entities,
            id: msg_id,
            reply_to_msg_id,
            date_unix,
            edit_date_unix,
            views: views_raw.clamp(0, i64::from(u32::MAX)) as u32,
            flags: MessageFlags::from_bits_truncate(flags_raw as u32),
        })
    }

    fn decode_deleted_message_row(row: &Row<'_>) -> Result<AyuDeletedMessage, rusqlite::Error> {
        let author: Option<String> = row.get(12)?;
        let text: String = row.get(13)?;
        let blob: Vec<u8> = row.get(14)?;
        let entities = Self::decode_entities(&blob).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Blob,
                Box::new(err),
            )
        })?;
        let msg = CompactMessage {
            dialog_id: DialogId(row.get(1)?),
            sender_id: PeerId(row.get::<_, i64>(4)? as u64),
            grouped_id: row.get::<_, i64>(5)? as u64,
            text: text.into_boxed_str(),
            author_name: author.map(|s| Arc::from(s.into_boxed_str())),
            media: None,
            entities,
            id: MsgId(row.get(3)?),
            reply_to_msg_id: MsgId(row.get(6)?),
            date_unix: row.get(7)?,
            edit_date_unix: row.get(8)?,
            views: row.get::<_, i64>(9)?.clamp(0, i64::from(u32::MAX)) as u32,
            flags: MessageFlags::from_bits_truncate(row.get::<_, i64>(10)? as u32)
                | MessageFlags::AYU_DELETED,
        };
        Ok(AyuDeletedMessage {
            user_id: UserId(row.get::<_, i64>(0)? as u64),
            topic_id: TopicId(row.get(2)?),
            entity_create_date: row.get(11)?,
            message: msg,
        })
    }

    fn decode_edited_revision_row(row: &Row<'_>) -> Result<AyuEditedRevision, rusqlite::Error> {
        let author: Option<String> = row.get(11)?;
        let text: String = row.get(12)?;
        let blob: Vec<u8> = row.get(13)?;
        let entities = Self::decode_entities(&blob).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Blob,
                Box::new(err),
            )
        })?;
        let msg = CompactMessage {
            dialog_id: DialogId(row.get(1)?),
            sender_id: PeerId(row.get::<_, i64>(3)? as u64),
            grouped_id: row.get::<_, i64>(4)? as u64,
            text: text.into_boxed_str(),
            author_name: author.map(|s| Arc::from(s.into_boxed_str())),
            media: None,
            entities,
            id: MsgId(row.get(2)?),
            reply_to_msg_id: MsgId(row.get(5)?),
            date_unix: row.get(6)?,
            edit_date_unix: row.get(7)?,
            views: row.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
            flags: MessageFlags::from_bits_truncate(row.get::<_, i64>(9)? as u32)
                | MessageFlags::AYU_EDITED,
        };
        Ok(AyuEditedRevision {
            user_id: UserId(row.get::<_, i64>(0)? as u64),
            entity_create_date: row.get(10)?,
            message: msg,
        })
    }

    fn upsert_single_conn(
        conn: &Connection,
        account_id: AccountId,
        msg: &CompactMessage,
    ) -> Result<(), rusqlite::Error> {
        let mut stmt = conn.prepare_cached(
            "
            INSERT INTO messages (
                account_id, dialog_id, msg_id, sender_id, grouped_id, reply_to_msg_id,
                date_unix, edit_date_unix, views, flags, author_name, text, entities,
                media_kind, media_file_id, media_access_hash, media_size_bytes,
                media_mime_type, media_local_path, media_thumb_path,
                media_width, media_height, media_duration_secs, media_has_spoiler
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17,
                ?18, ?19, ?20,
                ?21, ?22, ?23, ?24
            )
            ON CONFLICT(account_id, dialog_id, msg_id) DO UPDATE SET
                sender_id = excluded.sender_id,
                grouped_id = excluded.grouped_id,
                reply_to_msg_id = excluded.reply_to_msg_id,
                date_unix = excluded.date_unix,
                edit_date_unix = excluded.edit_date_unix,
                views = excluded.views,
                flags = excluded.flags,
                author_name = excluded.author_name,
                text = excluded.text,
                entities = excluded.entities,
                media_kind = excluded.media_kind,
                media_file_id = excluded.media_file_id,
                media_access_hash = excluded.media_access_hash,
                media_size_bytes = excluded.media_size_bytes,
                media_mime_type = excluded.media_mime_type,
                media_local_path = excluded.media_local_path,
                media_thumb_path = excluded.media_thumb_path,
                media_width = excluded.media_width,
                media_height = excluded.media_height,
                media_duration_secs = excluded.media_duration_secs,
                media_has_spoiler = excluded.media_has_spoiler
            ",
        )?;

        let entities_blob: &[u8] = bytemuck::cast_slice(msg.entities.as_slice());
        let (
            media_kind,
            media_file_id,
            media_access_hash,
            media_size_bytes,
            media_mime_type,
            media_local_path,
            media_thumb_path,
            media_width,
            media_height,
            media_duration_secs,
            media_has_spoiler,
        ) = match msg.media.as_deref() {
            Some(m) => (
                i64::from(m.kind as u8),
                Some(m.file_id as i64),
                Some(m.access_hash as i64),
                Some(m.size_bytes as i64),
                Some(m.mime_type.as_ref()),
                m.local_path.as_deref(),
                m.thumb_path.as_deref(),
                Some(i64::from(m.width)),
                Some(i64::from(m.height)),
                Some(i64::from(m.duration_secs)),
                Some(i64::from(u8::from(m.has_spoiler))),
            ),
            None => (
                0, None, None, None, None, None, None, None, None, None, None,
            ),
        };

        stmt.execute(params![
            account_id.0 as i64,
            msg.dialog_id.0,
            msg.id.0,
            msg.sender_id.0 as i64,
            msg.grouped_id as i64,
            msg.reply_to_msg_id.0,
            msg.date_unix,
            msg.edit_date_unix,
            i64::from(msg.views),
            i64::from(msg.flags.bits()),
            msg.author_name.as_deref(),
            msg.text.as_ref(),
            entities_blob,
            media_kind,
            media_file_id,
            media_access_hash,
            media_size_bytes,
            media_mime_type,
            media_local_path,
            media_thumb_path,
            media_width,
            media_height,
            media_duration_secs,
            media_has_spoiler,
        ])?;
        Ok(())
    }

    fn upsert_single_dialog_conn(
        conn: &Connection,
        account_id: AccountId,
        dialog: &CompactDialog,
    ) -> Result<(), rusqlite::Error> {
        let mut stmt = conn.prepare_cached(
            "
            INSERT INTO dialogs (
                account_id, dialog_id, peer_id, folder_id, title, draft_text,
                top_message_id, read_inbox_max_id, read_outbox_max_id,
                last_message_date, unread_count, unread_mentions_count, flags
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
            )
            ON CONFLICT(account_id, dialog_id) DO UPDATE SET
                peer_id = excluded.peer_id,
                folder_id = excluded.folder_id,
                title = excluded.title,
                draft_text = excluded.draft_text,
                top_message_id = excluded.top_message_id,
                read_inbox_max_id = excluded.read_inbox_max_id,
                read_outbox_max_id = excluded.read_outbox_max_id,
                last_message_date = excluded.last_message_date,
                unread_count = excluded.unread_count,
                unread_mentions_count = excluded.unread_mentions_count,
                flags = excluded.flags
            ",
        )?;
        stmt.execute(params![
            account_id.0 as i64,
            dialog.dialog_id.0,
            dialog.peer_id.0 as i64,
            dialog.folder_id,
            dialog.title.as_ref(),
            dialog.draft_text.as_deref(),
            dialog.top_message_id.0,
            dialog.read_inbox_max_id.0,
            dialog.read_outbox_max_id.0,
            dialog.last_message_date,
            i64::from(dialog.unread_count),
            i64::from(dialog.unread_mentions_count),
            i64::from(dialog.flags.bits()),
        ])?;
        Ok(())
    }

    fn decode_dialog_row(row: &Row<'_>) -> Result<CompactDialog, rusqlite::Error> {
        let title: String = row.get(3)?;
        let draft: Option<String> = row.get(4)?;
        let unread_count: i64 = row.get(9)?;
        let unread_mentions: i64 = row.get(10)?;
        let flags_raw: i64 = row.get(11)?;
        Ok(CompactDialog {
            dialog_id: DialogId(row.get(0)?),
            peer_id: PeerId(row.get::<_, i64>(1)? as u64),
            folder_id: row.get(2)?,
            title: title.into_boxed_str(),
            draft_text: draft.map(String::into_boxed_str),
            top_message_id: MsgId(row.get(5)?),
            read_inbox_max_id: MsgId(row.get(6)?),
            read_outbox_max_id: MsgId(row.get(7)?),
            last_message_date: row.get(8)?,
            unread_count: unread_count.clamp(0, i64::from(u32::MAX)) as u32,
            unread_mentions_count: unread_mentions.clamp(0, i64::from(u32::MAX)) as u32,
            flags: DialogFlags::from_bits_truncate(flags_raw as u16),
        })
    }

    fn escape_like_pattern(query: &str) -> String {
        let mut escaped = String::with_capacity(query.len().saturating_add(4));
        escaped.push('%');
        for ch in query.chars() {
            if matches!(ch, '%' | '_' | '\\') {
                escaped.push('\\');
            }
            escaped.push(ch);
        }
        escaped.push('%');
        escaped
    }
}

impl MessageRepository for SqliteStorage {
    fn upsert_message(
        &self,
        account_id: AccountId,
        message: &CompactMessage,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        Self::upsert_single_conn(&guard, account_id, message)
            .map_err(StorageError::from)
            .map_err(CoreError::from)
    }

    fn upsert_batch(
        &self,
        account_id: AccountId,
        messages: &[CompactMessage],
    ) -> Result<usize, CoreError> {
        let mut guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let tx = guard
            .transaction()
            .map_err(StorageError::from)
            .map_err(CoreError::from)?;
        for msg in messages {
            Self::upsert_single_conn(&tx, account_id, msg)
                .map_err(StorageError::from)
                .map_err(CoreError::from)?;
        }
        tx.commit()
            .map_err(StorageError::from)
            .map_err(CoreError::from)?;
        Ok(messages.len())
    }

    fn load_slice_around(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        center_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let before_limit = limit / 2;
        let after_limit = limit.saturating_sub(before_limit);

        let mut older = {
            let mut stmt = guard
                .prepare_cached(
                    "
                    SELECT dialog_id, msg_id, sender_id, grouped_id, reply_to_msg_id,
                           date_unix, edit_date_unix, views, flags, author_name, text, entities,
                           media_kind, media_file_id, media_access_hash, media_size_bytes,
                           media_mime_type, media_local_path, media_thumb_path,
                           media_width, media_height, media_duration_secs, media_has_spoiler
                    FROM messages
                    WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id < ?3
                    ORDER BY msg_id DESC
                    LIMIT ?4
                    ",
                )
                .map_err(StorageError::from)?;
            let rows = stmt
                .query_map(
                    params![
                        account_id.0 as i64,
                        dialog_id.0,
                        center_msg_id.0,
                        before_limit as i64
                    ],
                    Self::decode_message_row,
                )
                .map_err(StorageError::from)?;
            let mut vec = Vec::with_capacity(before_limit);
            for row in rows {
                vec.push(row.map_err(StorageError::from)?);
            }
            vec.reverse();
            vec
        };

        let mut newer = {
            let mut stmt = guard
                .prepare_cached(
                    "
                    SELECT dialog_id, msg_id, sender_id, grouped_id, reply_to_msg_id,
                           date_unix, edit_date_unix, views, flags, author_name, text, entities,
                           media_kind, media_file_id, media_access_hash, media_size_bytes,
                           media_mime_type, media_local_path, media_thumb_path,
                           media_width, media_height, media_duration_secs, media_has_spoiler
                    FROM messages
                    WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id >= ?3
                    ORDER BY msg_id ASC
                    LIMIT ?4
                    ",
                )
                .map_err(StorageError::from)?;
            let rows = stmt
                .query_map(
                    params![
                        account_id.0 as i64,
                        dialog_id.0,
                        center_msg_id.0,
                        after_limit as i64
                    ],
                    Self::decode_message_row,
                )
                .map_err(StorageError::from)?;
            let mut vec = Vec::with_capacity(after_limit);
            for row in rows {
                vec.push(row.map_err(StorageError::from)?);
            }
            vec
        };

        older.append(&mut newer);
        Ok(older)
    }

    fn load_older_than(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        before_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT dialog_id, msg_id, sender_id, grouped_id, reply_to_msg_id,
                       date_unix, edit_date_unix, views, flags, author_name, text, entities,
                       media_kind, media_file_id, media_access_hash, media_size_bytes,
                       media_mime_type, media_local_path, media_thumb_path,
                       media_width, media_height, media_duration_secs, media_has_spoiler
                FROM messages
                WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id < ?3
                ORDER BY msg_id DESC
                LIMIT ?4
                ",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                params![
                    account_id.0 as i64,
                    dialog_id.0,
                    before_msg_id.0,
                    limit as i64
                ],
                Self::decode_message_row,
            )
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(limit.min(128));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        out.reverse();
        Ok(out)
    }

    fn load_newer_than(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        after_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT dialog_id, msg_id, sender_id, grouped_id, reply_to_msg_id,
                       date_unix, edit_date_unix, views, flags, author_name, text, entities,
                       media_kind, media_file_id, media_access_hash, media_size_bytes,
                       media_mime_type, media_local_path, media_thumb_path,
                       media_width, media_height, media_duration_secs, media_has_spoiler
                FROM messages
                WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id > ?3
                ORDER BY msg_id ASC
                LIMIT ?4
                ",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                params![
                    account_id.0 as i64,
                    dialog_id.0,
                    after_msg_id.0,
                    limit as i64
                ],
                Self::decode_message_row,
            )
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(limit.min(128));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn delete_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        msg_ids: &[MsgId],
    ) -> Result<usize, CoreError> {
        let mut guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let tx = guard.transaction().map_err(StorageError::from)?;
        let mut deleted = 0usize;
        {
            let mut stmt = tx
                .prepare_cached(
                    "DELETE FROM messages WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id = ?3",
                )
                .map_err(StorageError::from)?;
            for id in msg_ids {
                deleted += stmt
                    .execute(params![account_id.0 as i64, dialog_id.0, id.0])
                    .map_err(StorageError::from)?;
            }
        }
        tx.commit().map_err(StorageError::from)?;
        Ok(deleted)
    }

    fn delete_messages_up_to(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        max_msg_id: MsgId,
    ) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "DELETE FROM messages WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id <= ?3",
            )
            .map_err(StorageError::from)?;
        let removed = stmt
            .execute(params![account_id.0 as i64, dialog_id.0, max_msg_id.0])
            .map_err(StorageError::from)?;
        Ok(removed)
    }

    fn search_dialog_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = Self::escape_like_pattern(query);
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT dialog_id, msg_id, sender_id, grouped_id, reply_to_msg_id,
                       date_unix, edit_date_unix, views, flags, author_name, text, entities,
                       media_kind, media_file_id, media_access_hash, media_size_bytes,
                       media_mime_type, media_local_path, media_thumb_path,
                       media_width, media_height, media_duration_secs, media_has_spoiler
                FROM messages
                WHERE account_id = ?1 AND dialog_id = ?2 AND text LIKE ?3 ESCAPE '\\'
                ORDER BY msg_id DESC
                LIMIT ?4
                ",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                params![account_id.0 as i64, dialog_id.0, pattern, limit as i64],
                Self::decode_message_row,
            )
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(limit.min(64));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn dialog_message_count(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
    ) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND dialog_id = ?2",
                params![account_id.0 as i64, dialog_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?
            .unwrap_or(0);
        Ok(count.max(0) as usize)
    }
}

impl DialogRepository for SqliteStorage {
    fn upsert_dialog(
        &self,
        account_id: AccountId,
        dialog: &CompactDialog,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        Self::upsert_single_dialog_conn(&guard, account_id, dialog)
            .map_err(StorageError::from)
            .map_err(CoreError::from)
    }

    fn upsert_dialog_batch(
        &self,
        account_id: AccountId,
        dialogs: &[CompactDialog],
    ) -> Result<usize, CoreError> {
        let mut guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let tx = guard.transaction().map_err(StorageError::from)?;
        for dialog in dialogs {
            Self::upsert_single_dialog_conn(&tx, account_id, dialog)
                .map_err(StorageError::from)?;
        }
        tx.commit().map_err(StorageError::from)?;
        Ok(dialogs.len())
    }

    fn update_dialog_read_state(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        read_inbox_max_id: MsgId,
        unread_count: u32,
    ) -> Result<bool, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                UPDATE dialogs
                SET read_inbox_max_id = ?3, unread_count = ?4
                WHERE account_id = ?1 AND dialog_id = ?2
                ",
            )
            .map_err(StorageError::from)?;
        let changed = stmt
            .execute(params![
                account_id.0 as i64,
                dialog_id.0,
                read_inbox_max_id.0,
                i64::from(unread_count),
            ])
            .map_err(StorageError::from)?;
        Ok(changed > 0)
    }

    fn remove_dialog(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
    ) -> Result<bool, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("DELETE FROM dialogs WHERE account_id = ?1 AND dialog_id = ?2")
            .map_err(StorageError::from)?;
        let removed = stmt
            .execute(params![account_id.0 as i64, dialog_id.0])
            .map_err(StorageError::from)?;
        Ok(removed > 0)
    }

    fn load_dialog_slice(
        &self,
        account_id: AccountId,
        folder_id: i32,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CompactDialog>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT dialog_id, peer_id, folder_id, title, draft_text,
                       top_message_id, read_inbox_max_id, read_outbox_max_id,
                       last_message_date, unread_count, unread_mentions_count, flags
                FROM dialogs
                WHERE account_id = ?1 AND folder_id = ?2
                ORDER BY (flags & 1) DESC, last_message_date DESC
                LIMIT ?3 OFFSET ?4
                ",
            )
            .map_err(StorageError::from)?;

        let rows = stmt
            .query_map(
                params![account_id.0 as i64, folder_id, limit as i64, offset as i64],
                Self::decode_dialog_row,
            )
            .map_err(StorageError::from)?;

        let mut out = Vec::with_capacity(limit.min(64));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn search_dialogs(
        &self,
        account_id: AccountId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactDialog>, CoreError> {
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = Self::escape_like_pattern(query);
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT dialog_id, peer_id, folder_id, title, draft_text,
                       top_message_id, read_inbox_max_id, read_outbox_max_id,
                       last_message_date, unread_count, unread_mentions_count, flags
                FROM dialogs
                WHERE account_id = ?1 AND title LIKE ?2 ESCAPE '\\'
                ORDER BY (flags & 1) DESC, last_message_date DESC
                LIMIT ?3
                ",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                params![account_id.0 as i64, pattern, limit as i64],
                Self::decode_dialog_row,
            )
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(limit.min(64));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn dialog_count(&self, account_id: AccountId, folder_id: i32) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let count: i64 = guard
            .query_row(
                "SELECT COUNT(*) FROM dialogs WHERE account_id = ?1 AND folder_id = ?2",
                params![account_id.0 as i64, folder_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?
            .unwrap_or(0);
        Ok(count.max(0) as usize)
    }
}

impl AyuAuditRepository for SqliteStorage {
    fn record_deleted_message(
        &self,
        account_id: AccountId,
        entry: &AyuDeletedMessage,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                INSERT INTO ayu_deleted_messages (
                    account_id, user_id, dialog_id, topic_id, msg_id, sender_id, grouped_id,
                    reply_to_msg_id, date_unix, edit_date_unix, views, flags,
                    entity_create_date, author_name, text, entities
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16
                )
                ",
            )
            .map_err(StorageError::from)?;
        let entities_blob: &[u8] = bytemuck::cast_slice(entry.message.entities.as_slice());
        stmt.execute(params![
            account_id.0 as i64,
            entry.user_id.0 as i64,
            entry.message.dialog_id.0,
            entry.topic_id.0,
            entry.message.id.0,
            entry.message.sender_id.0 as i64,
            entry.message.grouped_id as i64,
            entry.message.reply_to_msg_id.0,
            entry.message.date_unix,
            entry.message.edit_date_unix,
            i64::from(entry.message.views),
            i64::from(entry.message.flags.bits()),
            entry.entity_create_date,
            entry.message.author_name.as_deref(),
            entry.message.text.as_ref(),
            entities_blob,
        ])
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
        limit: usize,
    ) -> Result<Vec<AyuDeletedMessage>, CoreError> {
        self.query_deleted_messages(
            account_id,
            dialog_id,
            topic_id,
            MsgId(0),
            MsgId(0),
            limit,
            "",
        )
    }

    fn has_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
    ) -> Result<bool, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let exists: Option<i64> = guard
            .query_row(
                "
                SELECT 1 FROM ayu_deleted_messages
                WHERE account_id = ?1 AND dialog_id = ?2 AND (topic_id = ?3 OR ?3 = 0)
                LIMIT 1
                ",
                params![account_id.0 as i64, dialog_id.0, topic_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?;
        Ok(exists.is_some())
    }

    fn record_edited_revision(
        &self,
        _account_id: AccountId,
        entry: &AyuEditedRevision,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                INSERT INTO ayu_edited_messages (
                    user_id, dialog_id, msg_id, sender_id, grouped_id,
                    reply_to_msg_id, date_unix, edit_date_unix, views, flags,
                    entity_create_date, author_name, text, entities
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14
                )
                ",
            )
            .map_err(StorageError::from)?;
        let entities_blob: &[u8] = bytemuck::cast_slice(entry.message.entities.as_slice());
        stmt.execute(params![
            entry.user_id.0 as i64,
            entry.message.dialog_id.0,
            entry.message.id.0,
            entry.message.sender_id.0 as i64,
            entry.message.grouped_id as i64,
            entry.message.reply_to_msg_id.0,
            entry.message.date_unix,
            entry.message.edit_date_unix,
            i64::from(entry.message.views),
            i64::from(entry.message.flags.bits()),
            entry.entity_create_date,
            entry.message.author_name.as_deref(),
            entry.message.text.as_ref(),
            entities_blob,
        ])
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<AyuEditedRevision>, CoreError> {
        self.query_edited_revisions(user_id, dialog_id, msg_id, 0, 0, limit)
    }

    fn has_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
    ) -> Result<bool, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let exists: Option<i64> = guard
            .query_row(
                "
                SELECT 1 FROM ayu_edited_messages
                WHERE user_id = ?1 AND dialog_id = ?2 AND msg_id = ?3
                LIMIT 1
                ",
                params![user_id.0 as i64, dialog_id.0, msg_id.0],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?;
        Ok(exists.is_some())
    }

    fn save_regex_filter(&self, rule: &RegexFilterRule) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                INSERT INTO ayu_regex_filters (
                    id, pattern, dialog_id, enabled, reversed, case_insensitive
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(id) DO UPDATE SET
                    pattern = excluded.pattern,
                    dialog_id = excluded.dialog_id,
                    enabled = excluded.enabled,
                    reversed = excluded.reversed,
                    case_insensitive = excluded.case_insensitive
                ",
            )
            .map_err(StorageError::from)?;
        stmt.execute(params![
            rule.id.as_ref(),
            rule.pattern.as_ref(),
            rule.dialog_id.map(|d| d.0),
            i64::from(u8::from(rule.enabled)),
            i64::from(u8::from(rule.reversed)),
            i64::from(u8::from(rule.case_insensitive)),
        ])
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_regex_filters(&self) -> Result<Vec<RegexFilterRule>, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "SELECT id, pattern, dialog_id, enabled, reversed, case_insensitive FROM ayu_regex_filters",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map([], |row| {
                let id: Vec<u8> = row.get(0)?;
                let pattern: String = row.get(1)?;
                let dialog_id: Option<i64> = row.get(2)?;
                let enabled: i64 = row.get(3)?;
                let reversed: i64 = row.get(4)?;
                let case_insensitive: i64 = row.get(5)?;
                Ok(RegexFilterRule {
                    id: id.into_boxed_slice(),
                    pattern: pattern.into_boxed_str(),
                    dialog_id: dialog_id.map(DialogId),
                    enabled: enabled != 0,
                    reversed: reversed != 0,
                    case_insensitive: case_insensitive != 0,
                })
            })
            .map_err(StorageError::from)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn query_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
        min_msg_id: MsgId,
        max_msg_id: MsgId,
        limit: usize,
        search_query: &str,
    ) -> Result<Vec<AyuDeletedMessage>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let trimmed = search_query.trim();
        let like_pattern = (!trimmed.is_empty()).then(|| Self::escape_like_pattern(trimmed));
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT user_id, dialog_id, topic_id, msg_id, sender_id, grouped_id,
                       reply_to_msg_id, date_unix, edit_date_unix, views, flags,
                       entity_create_date, author_name, text, entities
                FROM ayu_deleted_messages
                WHERE account_id = ?1
                  AND dialog_id = ?2
                  AND (topic_id = ?3 OR ?3 = 0)
                  AND (msg_id > ?4 OR ?4 = 0)
                  AND (msg_id < ?5 OR ?5 = 0)
                  AND (?6 IS NULL OR text LIKE ?6 ESCAPE '\\')
                ORDER BY msg_id DESC, fake_id DESC
                LIMIT ?7
                ",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                params![
                    account_id.0 as i64,
                    dialog_id.0,
                    topic_id.0,
                    min_msg_id.0,
                    max_msg_id.0,
                    like_pattern,
                    limit as i64,
                ],
                Self::decode_deleted_message_row,
            )
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(limit.min(64));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn remove_deleted_message(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        msg_id: MsgId,
    ) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "DELETE FROM ayu_deleted_messages WHERE account_id = ?1 AND dialog_id = ?2 AND msg_id = ?3",
            )
            .map_err(StorageError::from)?;
        let deleted = stmt
            .execute(params![account_id.0 as i64, dialog_id.0, msg_id.0])
            .map_err(StorageError::from)?;
        Ok(deleted)
    }

    fn clear_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
    ) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "DELETE FROM ayu_deleted_messages WHERE account_id = ?1 AND dialog_id = ?2 AND (topic_id = ?3 OR ?3 = 0)",
            )
            .map_err(StorageError::from)?;
        let deleted = stmt
            .execute(params![account_id.0 as i64, dialog_id.0, topic_id.0])
            .map_err(StorageError::from)?;
        Ok(deleted)
    }

    fn query_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
        min_id: i64,
        max_id: i64,
        limit: usize,
    ) -> Result<Vec<AyuEditedRevision>, CoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                SELECT user_id, dialog_id, msg_id, sender_id, grouped_id,
                       reply_to_msg_id, date_unix, edit_date_unix, views, flags,
                       entity_create_date, author_name, text, entities
                FROM ayu_edited_messages
                WHERE user_id = ?1
                  AND dialog_id = ?2
                  AND msg_id = ?3
                  AND (fake_id > ?4 OR ?4 = 0)
                  AND (fake_id < ?5 OR ?5 = 0)
                ORDER BY fake_id DESC
                LIMIT ?6
                ",
            )
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map(
                params![
                    user_id.0 as i64,
                    dialog_id.0,
                    msg_id.0,
                    min_id,
                    max_id,
                    limit as i64,
                ],
                Self::decode_edited_revision_row,
            )
            .map_err(StorageError::from)?;
        let mut out = Vec::with_capacity(limit.min(32));
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn delete_regex_filter(&self, filter_id: &[u8]) -> Result<bool, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("DELETE FROM ayu_regex_filters WHERE id = ?1")
            .map_err(StorageError::from)?;
        let deleted = stmt.execute(params![filter_id]).map_err(StorageError::from)?;
        Ok(deleted > 0)
    }

    fn delete_all_regex_filters(&self) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("DELETE FROM ayu_regex_filters")
            .map_err(StorageError::from)?;
        let deleted = stmt.execute([]).map_err(StorageError::from)?;
        Ok(deleted)
    }

    fn save_regex_exclusion(
        &self,
        exclusion: &RegexFilterGlobalExclusion,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "INSERT OR IGNORE INTO ayu_regex_exclusions (dialog_id, filter_id) VALUES (?1, ?2)",
            )
            .map_err(StorageError::from)?;
        stmt.execute(params![exclusion.dialog_id.0, exclusion.filter_id.as_ref()])
            .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_regex_exclusions(&self) -> Result<Vec<RegexFilterGlobalExclusion>, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("SELECT dialog_id, filter_id FROM ayu_regex_exclusions")
            .map_err(StorageError::from)?;
        let rows = stmt
            .query_map([], |row| {
                let dialog_id: i64 = row.get(0)?;
                let filter_id: Vec<u8> = row.get(1)?;
                Ok(RegexFilterGlobalExclusion {
                    dialog_id: DialogId(dialog_id),
                    filter_id: filter_id.into_boxed_slice(),
                })
            })
            .map_err(StorageError::from)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(StorageError::from)?);
        }
        Ok(out)
    }

    fn delete_regex_exclusion(
        &self,
        dialog_id: DialogId,
        filter_id: &[u8],
    ) -> Result<bool, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "DELETE FROM ayu_regex_exclusions WHERE dialog_id = ?1 AND filter_id = ?2",
            )
            .map_err(StorageError::from)?;
        let deleted = stmt
            .execute(params![dialog_id.0, filter_id])
            .map_err(StorageError::from)?;
        Ok(deleted > 0)
    }

    fn delete_exclusions_by_filter_id(&self, filter_id: &[u8]) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("DELETE FROM ayu_regex_exclusions WHERE filter_id = ?1")
            .map_err(StorageError::from)?;
        let deleted = stmt.execute(params![filter_id]).map_err(StorageError::from)?;
        Ok(deleted)
    }

    fn delete_all_regex_exclusions(&self) -> Result<usize, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("DELETE FROM ayu_regex_exclusions")
            .map_err(StorageError::from)?;
        let deleted = stmt.execute([]).map_err(StorageError::from)?;
        Ok(deleted)
    }
}

impl AuthStateRepository for SqliteStorage {
    fn save_account_auth(
        &self,
        account_id: AccountId,
        index: i32,
        is_test_dc: bool,
        is_premium: bool,
        auth_state: AuthState,
        blob: &MtpAuthorizationBlob,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                INSERT INTO account_auth (
                    account_id, account_index, user_id, main_dc_id,
                    is_test_dc, is_premium, auth_state, serialized_blob
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                ON CONFLICT(account_id) DO UPDATE SET
                    account_index = excluded.account_index,
                    user_id = excluded.user_id,
                    main_dc_id = excluded.main_dc_id,
                    is_test_dc = excluded.is_test_dc,
                    is_premium = excluded.is_premium,
                    auth_state = excluded.auth_state,
                    serialized_blob = excluded.serialized_blob
                ",
            )
            .map_err(StorageError::from)?;
        let encoded = blob.encode_tdesktop();
        stmt.execute(params![
            account_id.0 as i64,
            index,
            blob.user_id.0 as i64,
            blob.main_dc_id,
            i64::from(u8::from(is_test_dc)),
            i64::from(u8::from(is_premium)),
            i64::from(auth_state as u8),
            encoded,
        ])
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_account_auth(
        &self,
        account_id: AccountId,
    ) -> Result<Option<MtpAuthorizationBlob>, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "SELECT serialized_blob FROM account_auth WHERE account_id = ?1",
            )
            .map_err(StorageError::from)?;
        let raw_opt: Option<Vec<u8>> = stmt
            .query_row(params![account_id.0 as i64], |row| row.get(0))
            .optional()
            .map_err(StorageError::from)?;
        match raw_opt {
            Some(bytes) => Ok(Some(MtpAuthorizationBlob::decode_tdesktop(&bytes)?)),
            None => Ok(None),
        }
    }

    fn delete_account_auth(&self, account_id: AccountId) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached("DELETE FROM account_auth WHERE account_id = ?1")
            .map_err(StorageError::from)?;
        stmt.execute(params![account_id.0 as i64])
            .map_err(StorageError::from)?;
        Ok(())
    }
}

impl SyncStateRepository for SqliteStorage {
    fn save_sync_state(
        &self,
        account_id: AccountId,
        state: SyncStateSnapshot,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                INSERT INTO account_sync_state (
                    account_id, pts, qts, date_unix, seq
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(account_id) DO UPDATE SET
                    pts = excluded.pts,
                    qts = excluded.qts,
                    date_unix = excluded.date_unix,
                    seq = excluded.seq
                ",
            )
            .map_err(StorageError::from)?;
        stmt.execute(params![
            account_id.0 as i64,
            state.pts,
            state.qts,
            state.date,
            state.seq,
        ])
        .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_sync_state(&self, account_id: AccountId) -> Result<Option<SyncStateSnapshot>, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "SELECT pts, qts, date_unix, seq FROM account_sync_state WHERE account_id = ?1",
            )
            .map_err(StorageError::from)?;
        let row = stmt
            .query_row(params![account_id.0 as i64], |r| {
                Ok(SyncStateSnapshot {
                    pts: r.get(0)?,
                    qts: r.get(1)?,
                    date: r.get(2)?,
                    seq: r.get(3)?,
                })
            })
            .optional()
            .map_err(StorageError::from)?;
        Ok(row)
    }

    fn save_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
        pts: i32,
    ) -> Result<(), CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "
                INSERT INTO channel_sync_pts (
                    account_id, channel_id, pts
                ) VALUES (?1, ?2, ?3)
                ON CONFLICT(account_id, channel_id) DO UPDATE SET
                    pts = excluded.pts
                ",
            )
            .map_err(StorageError::from)?;
        stmt.execute(params![account_id.0 as i64, channel_id as i64, pts])
            .map_err(StorageError::from)?;
        Ok(())
    }

    fn load_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
    ) -> Result<Option<i32>, CoreError> {
        let guard = self.conn.lock().map_err(|_| StorageError::LockPoisoned)?;
        let mut stmt = guard
            .prepare_cached(
                "SELECT pts FROM channel_sync_pts WHERE account_id = ?1 AND channel_id = ?2",
            )
            .map_err(StorageError::from)?;
        let pts = stmt
            .query_row(params![account_id.0 as i64, channel_id as i64], |r| r.get(0))
            .optional()
            .map_err(StorageError::from)?;
        Ok(pts)
    }
}
