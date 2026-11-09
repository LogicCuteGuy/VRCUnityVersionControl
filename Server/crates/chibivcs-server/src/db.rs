use crate::blobs::BlobStore;
use anyhow::{bail, Context, Result};
use chibivcs_proto::*;
use rand::RngCore;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
}

#[derive(Debug, Error)]
pub enum VcsError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: admin required")]
    Forbidden,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("path '{path}' is locked by '{owner}'")]
    LockedByOther { path: String, owner: String },
    #[error("branch head moved to cs:{head_changeset_id} with overlapping changes; update workspace first")]
    NeedsUpdate { head_changeset_id: i64 },
    #[error("missing blob on server: {0}")]
    MissingBlob(String),
    #[error("invalid input: {0}")]
    BadRequest(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn hash_password(password: &str, salt_hex: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt_hex.as_bytes());
    h.update(b":chibivcs:");
    h.update(password.as_bytes());
    let mut out = h.finalize();
    for _ in 0..10_000 {
        let mut round = Sha256::new();
        round.update(&out);
        round.update(salt_hex.as_bytes());
        out = round.finalize();
    }
    hex::encode(out)
}

fn random_hex(bytes_len: usize) -> String {
    let mut buf = vec![0u8; bytes_len];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).context("open sqlite db")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS users (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                username TEXT NOT NULL UNIQUE COLLATE NOCASE,
                salt_hex TEXT NOT NULL,
                password_hash TEXT NOT NULL,
                is_admin INTEGER NOT NULL DEFAULT 0,
                created_at_unix INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tokens (
                token TEXT PRIMARY KEY,
                user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at_unix INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS repos (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                description TEXT NOT NULL DEFAULT '',
                created_at_unix INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS branches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
                name TEXT NOT NULL COLLATE NOCASE,
                head_changeset_id INTEGER NOT NULL DEFAULT 0,
                created_by TEXT NOT NULL,
                created_at_unix INTEGER NOT NULL,
                UNIQUE(repo_id, name)
            );

            CREATE TABLE IF NOT EXISTS changesets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
                branch TEXT NOT NULL,
                parent_id INTEGER NOT NULL DEFAULT 0,
                author TEXT NOT NULL,
                comment TEXT NOT NULL,
                timestamp_unix INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS changeset_files (
                changeset_id INTEGER NOT NULL REFERENCES changesets(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                kind TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                size INTEGER NOT NULL,
                PRIMARY KEY(changeset_id, path)
            );

            CREATE TABLE IF NOT EXISTS tree_entries (
                changeset_id INTEGER NOT NULL REFERENCES changesets(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                size INTEGER NOT NULL,
                PRIMARY KEY(changeset_id, path)
            );

            CREATE TABLE IF NOT EXISTS locks (
                repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
                branch TEXT NOT NULL COLLATE NOCASE,
                path TEXT NOT NULL,
                owner TEXT NOT NULL COLLATE NOCASE,
                locked_at_unix INTEGER NOT NULL,
                PRIMARY KEY(repo_id, branch, path)
            );
            ",
        )?;
        Ok(())
    }

    pub fn upsert_user(&self, username: &str, password: &str, is_admin: bool) -> Result<()> {
        let username = username.trim();
        if username.is_empty() {
            bail!("username cannot be empty");
        }
        let salt = random_hex(16);
        let pwd_hash = hash_password(password, &salt);
        let now = now_unix();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO users (username, salt_hex, password_hash, is_admin, created_at_unix)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(username) DO UPDATE SET
                salt_hex = excluded.salt_hex,
                password_hash = excluded.password_hash,
                is_admin = excluded.is_admin",
            params![username, salt, pwd_hash, if is_admin { 1 } else { 0 }, now],
        )?;
        Ok(())
    }

    pub fn create_token_for_user(&self, username: &str) -> Result<String> {
        let conn = self.conn.lock().unwrap();
        let user_id: i64 = conn
            .query_row(
                "SELECT id FROM users WHERE username = ?1",
                params![username],
                |r| r.get(0),
            )
            .context("user not found")?;
        let token = random_hex(32);
        conn.execute(
            "INSERT INTO tokens (token, user_id, created_at_unix) VALUES (?1, ?2, ?3)",
            params![token, user_id, now_unix()],
        )?;
        Ok(token)
    }

    pub fn list_users(&self) -> Result<Vec<(String, bool, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT username, is_admin, created_at_unix FROM users ORDER BY id ASC")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn verify_login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Option<LoginResponse>, VcsError> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(i64, String, String, String, i64)> = conn
            .query_row(
                "SELECT id, username, salt_hex, password_hash, is_admin FROM users WHERE username = ?1",
                params![username.trim()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|e| VcsError::Internal(e.into()))?;

        let Some((user_id, canonical_user, salt, expected_hash, is_admin)) = row else {
            return Ok(None);
        };

        let computed = hash_password(password, &salt);
        if computed != expected_hash {
            return Ok(None);
        }

        let token = random_hex(32);
        conn.execute(
            "INSERT INTO tokens (token, user_id, created_at_unix) VALUES (?1, ?2, ?3)",
            params![token, user_id, now_unix()],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;

        Ok(Some(LoginResponse {
            token,
            username: canonical_user,
            is_admin: is_admin != 0,
        }))
    }

    pub fn authenticate_token(&self, token: &str) -> Result<Option<AuthUser>> {
        let conn = self.conn.lock().unwrap();
        let user = conn
            .query_row(
                "SELECT u.id, u.username, u.is_admin
                 FROM tokens t
                 JOIN users u ON u.id = t.user_id
                 WHERE t.token = ?1",
                params![token.trim()],
                |r| {
                    Ok(AuthUser {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        is_admin: r.get::<_, i64>(2)? != 0,
                    })
                },
            )
            .optional()?;
        Ok(user)
    }

    pub fn create_repo(
        &self,
        name: &str,
        description: &str,
        creator: &str,
    ) -> Result<RepoInfo, VcsError> {
        let name = name.trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(VcsError::BadRequest(
                "repo name must be alphanumeric, '-' or '_'".into(),
            ));
        }

        let now = now_unix();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| VcsError::Internal(e.into()))?;

        tx.execute(
            "INSERT OR IGNORE INTO repos (name, description, created_at_unix) VALUES (?1, ?2, ?3)",
            params![name, description, now],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;

        let (repo_id, created_at): (i64, i64) = tx
            .query_row(
                "SELECT id, created_at_unix FROM repos WHERE name = ?1",
                params![name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| VcsError::Internal(e.into()))?;

        tx.execute(
            "INSERT OR IGNORE INTO branches (repo_id, name, head_changeset_id, created_by, created_at_unix)
             VALUES (?1, 'main', 0, ?2, ?3)",
            params![repo_id, creator, now],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;

        let head: i64 = tx
            .query_row(
                "SELECT head_changeset_id FROM branches WHERE repo_id = ?1 AND name = 'main'",
                params![repo_id],
                |r| r.get(0),
            )
            .unwrap_or(0);

        tx.commit().map_err(|e| VcsError::Internal(e.into()))?;

        Ok(RepoInfo {
            name: name.to_string(),
            description: description.to_string(),
            head_changeset_id: head,
            created_at_unix: created_at,
        })
    }

    pub fn list_repos(&self) -> Result<Vec<RepoInfo>, VcsError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT r.name, r.description, r.created_at_unix,
                        COALESCE((SELECT MAX(id) FROM changesets c WHERE c.repo_id = r.id), 0)
                 FROM repos r
                 ORDER BY r.name ASC",
            )
            .map_err(|e| VcsError::Internal(e.into()))?;

        let list = stmt
            .query_map([], |r| {
                Ok(RepoInfo {
                    name: r.get(0)?,
                    description: r.get(1)?,
                    created_at_unix: r.get(2)?,
                    head_changeset_id: r.get(3)?,
                })
            })
            .map_err(|e| VcsError::Internal(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| VcsError::Internal(e.into()))?;

        Ok(list)
    }

    fn get_repo_id(conn: &Connection, repo: &str) -> Result<i64, VcsError> {
        conn.query_row("SELECT id FROM repos WHERE name = ?1", params![repo], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| VcsError::Internal(e.into()))?
        .ok_or_else(|| VcsError::NotFound(format!("repository '{}'", repo)))
    }

    pub fn list_branches(&self, repo: &str) -> Result<Vec<BranchInfo>, VcsError> {
        let conn = self.conn.lock().unwrap();
        let repo_id = Self::get_repo_id(&conn, repo)?;
        let mut stmt = conn
            .prepare(
                "SELECT name, head_changeset_id, created_by, created_at_unix
                 FROM branches WHERE repo_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| VcsError::Internal(e.into()))?;
        let rows = stmt
            .query_map(params![repo_id], |r| {
                Ok(BranchInfo {
                    name: r.get(0)?,
                    head_changeset_id: r.get(1)?,
                    created_by: r.get(2)?,
                    created_at_unix: r.get(3)?,
                })
            })
            .map_err(|e| VcsError::Internal(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| VcsError::Internal(e.into()))?;
        Ok(rows)
    }

    pub fn create_branch(
        &self,
        repo: &str,
        branch_name: &str,
        from_changeset_id: i64,
        creator: &str,
    ) -> Result<BranchInfo, VcsError> {
        let branch_name = branch_name.trim().trim_matches('/');
        if branch_name.is_empty() {
            return Err(VcsError::BadRequest("branch name cannot be empty".into()));
        }
        let now = now_unix();
        let conn = self.conn.lock().unwrap();
        let repo_id = Self::get_repo_id(&conn, repo)?;

        if from_changeset_id > 0 {
            let exists: Option<i64> = conn
                .query_row(
                    "SELECT id FROM changesets WHERE id = ?1 AND repo_id = ?2",
                    params![from_changeset_id, repo_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| VcsError::Internal(e.into()))?;
            if exists.is_none() {
                return Err(VcsError::NotFound(format!(
                    "changeset cs:{}",
                    from_changeset_id
                )));
            }
        }

        conn.execute(
            "INSERT INTO branches (repo_id, name, head_changeset_id, created_by, created_at_unix)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo_id, name) DO NOTHING",
            params![repo_id, branch_name, from_changeset_id, creator, now],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;

        let info = conn
            .query_row(
                "SELECT name, head_changeset_id, created_by, created_at_unix
                 FROM branches WHERE repo_id = ?1 AND name = ?2",
                params![repo_id, branch_name],
                |r| {
                    Ok(BranchInfo {
                        name: r.get(0)?,
                        head_changeset_id: r.get(1)?,
                        created_by: r.get(2)?,
                        created_at_unix: r.get(3)?,
                    })
                },
            )
            .map_err(|e| VcsError::Internal(e.into()))?;

        Ok(info)
    }

    pub fn checkin(
        &self,
        repo: &str,
        author: &str,
        req: CheckinRequest,
        blobs: &BlobStore,
    ) -> Result<CheckinResponse, VcsError> {
        if req.changes.is_empty() {
            return Err(VcsError::BadRequest("checkin has no file changes".into()));
        }
        let branch = if req.branch.trim().is_empty() {
            "main".to_string()
        } else {
            req.branch.trim().trim_matches('/').to_string()
        };

        // Validate & normalize paths and check blob existence upfront
        let mut normalized_changes = Vec::with_capacity(req.changes.len());
        for ch in &req.changes {
            let Some(norm_path) = normalize_rel_path(&ch.path) else {
                return Err(VcsError::BadRequest(format!("invalid path: {}", ch.path)));
            };
            let sha = if ch.kind == ChangeKind::Deleted {
                String::new()
            } else {
                let clean = BlobStore::validate_hash(&ch.sha256)
                    .map_err(|e| VcsError::BadRequest(e.to_string()))?;
                if !blobs.has(&clean) {
                    return Err(VcsError::MissingBlob(clean));
                }
                clean
            };
            normalized_changes.push(FileChangeItem {
                path: norm_path,
                kind: ch.kind,
                sha256: sha,
                size: ch.size,
            });
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| VcsError::Internal(e.into()))?;
        let repo_id = Self::get_repo_id(&tx, repo)?;

        // Ensure branch exists
        let now = now_unix();
        tx.execute(
            "INSERT OR IGNORE INTO branches (repo_id, name, head_changeset_id, created_by, created_at_unix)
             VALUES (?1, ?2, 0, ?3, ?4)",
            params![repo_id, branch, author, now],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;

        let current_head: i64 = tx
            .query_row(
                "SELECT head_changeset_id FROM branches WHERE repo_id = ?1 AND name = ?2",
                params![repo_id, branch],
                |r| r.get(0),
            )
            .map_err(|e| VcsError::Internal(e.into()))?;

        // 1. Check locks on modified paths
        for ch in &normalized_changes {
            let lock_owner: Option<String> = tx
                .query_row(
                    "SELECT owner FROM locks WHERE repo_id = ?1 AND branch = ?2 AND path = ?3",
                    params![repo_id, branch, ch.path],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| VcsError::Internal(e.into()))?;

            if let Some(owner) = lock_owner {
                if !owner.eq_ignore_ascii_case(author) {
                    return Err(VcsError::LockedByOther {
                        path: ch.path.clone(),
                        owner,
                    });
                }
            }
        }

        // 2. Check if head moved since req.parent_changeset_id
        let mut rebased = false;
        if current_head != req.parent_changeset_id {
            if req.parent_changeset_id > current_head {
                return Err(VcsError::BadRequest(format!(
                    "parent changeset cs:{} is ahead of branch head cs:{}",
                    req.parent_changeset_id, current_head
                )));
            }

            // Collect all paths touched between parent_changeset_id and current_head on this branch
            let mut stmt = tx
                .prepare(
                    "SELECT DISTINCT cf.path
                     FROM changeset_files cf
                     JOIN changesets c ON c.id = cf.changeset_id
                     WHERE c.repo_id = ?1 AND c.branch = ?2 AND c.id > ?3 AND c.id <= ?4",
                )
                .map_err(|e| VcsError::Internal(e.into()))?;
            let touched: HashSet<String> = stmt
                .query_map(
                    params![repo_id, branch, req.parent_changeset_id, current_head],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| VcsError::Internal(e.into()))?
                .collect::<std::result::Result<HashSet<_>, _>>()
                .map_err(|e| VcsError::Internal(e.into()))?;
            drop(stmt);

            for ch in &normalized_changes {
                if touched.contains(&ch.path) {
                    return Err(VcsError::NeedsUpdate {
                        head_changeset_id: current_head,
                    });
                }
            }
            rebased = true;
        }

        // 3. Insert new changeset
        tx.execute(
            "INSERT INTO changesets (repo_id, branch, parent_id, author, comment, timestamp_unix)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![repo_id, branch, current_head, author, req.comment.trim(), now],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;
        let new_cs_id = tx.last_insert_rowid();

        // 4. Copy parent tree snapshot
        if current_head > 0 {
            tx.execute(
                "INSERT INTO tree_entries (changeset_id, path, sha256, size)
                 SELECT ?1, path, sha256, size FROM tree_entries WHERE changeset_id = ?2",
                params![new_cs_id, current_head],
            )
            .map_err(|e| VcsError::Internal(e.into()))?;
        }

        // 5. Apply changeset files & update tree_entries + release user's locks
        for ch in &normalized_changes {
            let kind_str = match ch.kind {
                ChangeKind::Added => "added",
                ChangeKind::Modified => "modified",
                ChangeKind::Deleted => "deleted",
            };
            tx.execute(
                "INSERT OR REPLACE INTO changeset_files (changeset_id, path, kind, sha256, size)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![new_cs_id, ch.path, kind_str, ch.sha256, ch.size as i64],
            )
            .map_err(|e| VcsError::Internal(e.into()))?;

            match ch.kind {
                ChangeKind::Added | ChangeKind::Modified => {
                    tx.execute(
                        "INSERT OR REPLACE INTO tree_entries (changeset_id, path, sha256, size)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![new_cs_id, ch.path, ch.sha256, ch.size as i64],
                    )
                    .map_err(|e| VcsError::Internal(e.into()))?;
                }
                ChangeKind::Deleted => {
                    tx.execute(
                        "DELETE FROM tree_entries WHERE changeset_id = ?1 AND path = ?2",
                        params![new_cs_id, ch.path],
                    )
                    .map_err(|e| VcsError::Internal(e.into()))?;
                }
            }

            // Release lock held by author on committed path
            tx.execute(
                "DELETE FROM locks WHERE repo_id = ?1 AND branch = ?2 AND path = ?3 AND owner = ?4",
                params![repo_id, branch, ch.path, author],
            )
            .map_err(|e| VcsError::Internal(e.into()))?;
        }

        // 6. Advance branch head
        tx.execute(
            "UPDATE branches SET head_changeset_id = ?1 WHERE repo_id = ?2 AND name = ?3",
            params![new_cs_id, repo_id, branch],
        )
        .map_err(|e| VcsError::Internal(e.into()))?;

        tx.commit().map_err(|e| VcsError::Internal(e.into()))?;

        Ok(CheckinResponse {
            changeset_id: new_cs_id,
            branch,
            rebased,
        })
    }

    pub fn list_changesets(
        &self,
        repo: &str,
        branch_filter: Option<&str>,
        after_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<ChangesetSummary>, VcsError> {
        let conn = self.conn.lock().unwrap();
        let repo_id = Self::get_repo_id(&conn, repo)?;
        let limit = limit.clamp(1, 500) as i64;
        let after = after_id.unwrap_or(0);

        let mut list = Vec::new();
        if let Some(b) = branch_filter.filter(|s| !s.trim().is_empty()) {
            let mut stmt = conn
                .prepare(
                    "SELECT c.id, c.branch, c.parent_id, c.author, c.comment, c.timestamp_unix,
                            (SELECT COUNT(*) FROM changeset_files cf WHERE cf.changeset_id = c.id)
                     FROM changesets c
                     WHERE c.repo_id = ?1 AND c.branch = ?2 AND c.id > ?3
                     ORDER BY c.id DESC
                     LIMIT ?4",
                )
                .map_err(|e| VcsError::Internal(e.into()))?;
            let rows = stmt
                .query_map(params![repo_id, b.trim(), after, limit], |r| {
                    Ok(ChangesetSummary {
                        id: r.get(0)?,
                        branch: r.get(1)?,
                        parent_id: r.get(2)?,
                        author: r.get(3)?,
                        comment: r.get(4)?,
                        timestamp_unix: r.get(5)?,
                        change_count: r.get::<_, i64>(6)? as usize,
                    })
                })
                .map_err(|e| VcsError::Internal(e.into()))?;
            for row in rows {
                list.push(row.map_err(|e| VcsError::Internal(e.into()))?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT c.id, c.branch, c.parent_id, c.author, c.comment, c.timestamp_unix,
                            (SELECT COUNT(*) FROM changeset_files cf WHERE cf.changeset_id = c.id)
                     FROM changesets c
                     WHERE c.repo_id = ?1 AND c.id > ?2
                     ORDER BY c.id DESC
                     LIMIT ?3",
                )
                .map_err(|e| VcsError::Internal(e.into()))?;
            let rows = stmt
                .query_map(params![repo_id, after, limit], |r| {
                    Ok(ChangesetSummary {
                        id: r.get(0)?,
                        branch: r.get(1)?,
                        parent_id: r.get(2)?,
                        author: r.get(3)?,
                        comment: r.get(4)?,
                        timestamp_unix: r.get(5)?,
                        change_count: r.get::<_, i64>(6)? as usize,
                    })
                })
                .map_err(|e| VcsError::Internal(e.into()))?;
            for row in rows {
                list.push(row.map_err(|e| VcsError::Internal(e.into()))?);
            }
        }
        Ok(list)
    }

    pub fn get_changeset(&self, repo: &str, cs_id: i64) -> Result<ChangesetDetail, VcsError> {
        let conn = self.conn.lock().unwrap();
        let repo_id = Self::get_repo_id(&conn, repo)?;

        let summary: Option<ChangesetSummary> = conn
            .query_row(
                "SELECT c.id, c.branch, c.parent_id, c.author, c.comment, c.timestamp_unix,
                        (SELECT COUNT(*) FROM changeset_files cf WHERE cf.changeset_id = c.id)
                 FROM changesets c
                 WHERE c.repo_id = ?1 AND c.id = ?2",
                params![repo_id, cs_id],
                |r| {
                    Ok(ChangesetSummary {
                        id: r.get(0)?,
                        branch: r.get(1)?,
                        parent_id: r.get(2)?,
                        author: r.get(3)?,
                        comment: r.get(4)?,
                        timestamp_unix: r.get(5)?,
                        change_count: r.get::<_, i64>(6)? as usize,
                    })
                },
            )
            .optional()
            .map_err(|e| VcsError::Internal(e.into()))?;

        let Some(summary) = summary else {
            return Err(VcsError::NotFound(format!("changeset cs:{}", cs_id)));
        };

        let mut stmt = conn
            .prepare(
                "SELECT path, kind, sha256, size
                 FROM changeset_files WHERE changeset_id = ?1 ORDER BY path ASC",
            )
            .map_err(|e| VcsError::Internal(e.into()))?;
        let changes = stmt
            .query_map(params![cs_id], |r| {
                let kind_str: String = r.get(1)?;
                let kind = match kind_str.as_str() {
                    "added" => ChangeKind::Added,
                    "deleted" => ChangeKind::Deleted,
                    _ => ChangeKind::Modified,
                };
                Ok(FileChangeItem {
                    path: r.get(0)?,
                    kind,
                    sha256: r.get(2)?,
                    size: r.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| VcsError::Internal(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| VcsError::Internal(e.into()))?;

        Ok(ChangesetDetail { summary, changes })
    }

    /// Returns the full file tree for a changeset ID (or 0 for empty tree, or -1 for latest on branch).
    pub fn get_tree(
        &self,
        repo: &str,
        changeset_id: i64,
        branch_opt: Option<&str>,
    ) -> Result<TreeResponse, VcsError> {
        let conn = self.conn.lock().unwrap();
        let repo_id = Self::get_repo_id(&conn, repo)?;
        let branch_name = branch_opt.unwrap_or("main").trim();

        let target_cs = if changeset_id < 0 {
            conn.query_row(
                "SELECT head_changeset_id FROM branches WHERE repo_id = ?1 AND name = ?2",
                params![repo_id, branch_name],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(|e| VcsError::Internal(e.into()))?
            .unwrap_or(0)
        } else {
            changeset_id
        };

        if target_cs == 0 {
            return Ok(TreeResponse {
                changeset_id: 0,
                branch: branch_name.to_string(),
                files: Vec::new(),
            });
        }

        let cs_branch: Option<String> = conn
            .query_row(
                "SELECT branch FROM changesets WHERE repo_id = ?1 AND id = ?2",
                params![repo_id, target_cs],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| VcsError::Internal(e.into()))?;

        let Some(cs_branch) = cs_branch else {
            return Err(VcsError::NotFound(format!("changeset cs:{}", target_cs)));
        };

        let mut stmt = conn
            .prepare(
                "SELECT path, sha256, size
                 FROM tree_entries WHERE changeset_id = ?1 ORDER BY path ASC",
            )
            .map_err(|e| VcsError::Internal(e.into()))?;

        let files = stmt
            .query_map(params![target_cs], |r| {
                Ok(FileEntry {
                    path: r.get(0)?,
                    sha256: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(|e| VcsError::Internal(e.into()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| VcsError::Internal(e.into()))?;

        Ok(TreeResponse {
            changeset_id: target_cs,
            branch: cs_branch,
            files,
        })
    }

    pub fn list_locks(
        &self,
        repo: &str,
        branch_opt: Option<&str>,
    ) -> Result<Vec<LockInfo>, VcsError> {
        let conn = self.conn.lock().unwrap();
        let repo_id = Self::get_repo_id(&conn, repo)?;

        let mut list = Vec::new();
        if let Some(b) = branch_opt.filter(|s| !s.trim().is_empty()) {
            let mut stmt = conn
                .prepare(
                    "SELECT path, branch, owner, locked_at_unix
                     FROM locks WHERE repo_id = ?1 AND branch = ?2 ORDER BY path ASC",
                )
                .map_err(|e| VcsError::Internal(e.into()))?;
            let rows = stmt
                .query_map(params![repo_id, b.trim()], |r| {
                    Ok(LockInfo {
                        path: r.get(0)?,
                        branch: r.get(1)?,
                        owner: r.get(2)?,
                        locked_at_unix: r.get(3)?,
                    })
                })
                .map_err(|e| VcsError::Internal(e.into()))?;
            for row in rows {
                list.push(row.map_err(|e| VcsError::Internal(e.into()))?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT path, branch, owner, locked_at_unix
                     FROM locks WHERE repo_id = ?1 ORDER BY path ASC",
                )
                .map_err(|e| VcsError::Internal(e.into()))?;
            let rows = stmt
                .query_map(params![repo_id], |r| {
                    Ok(LockInfo {
                        path: r.get(0)?,
                        branch: r.get(1)?,
                        owner: r.get(2)?,
                        locked_at_unix: r.get(3)?,
                    })
                })
                .map_err(|e| VcsError::Internal(e.into()))?;
            for row in rows {
                list.push(row.map_err(|e| VcsError::Internal(e.into()))?);
            }
        }
        Ok(list)
    }

    pub fn acquire_locks(
        &self,
        repo: &str,
        branch: &str,
        owner: &str,
        paths: &[String],
    ) -> Result<Vec<LockInfo>, VcsError> {
        let branch = if branch.trim().is_empty() {
            "main"
        } else {
            branch.trim()
        };
        let mut norm_paths = Vec::with_capacity(paths.len());
        for p in paths {
            let Some(np) = normalize_rel_path(p) else {
                return Err(VcsError::BadRequest(format!("invalid path: {}", p)));
            };
            norm_paths.push(np);
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| VcsError::Internal(e.into()))?;
        let repo_id = Self::get_repo_id(&tx, repo)?;
        let now = now_unix();

        for np in &norm_paths {
            let existing: Option<String> = tx
                .query_row(
                    "SELECT owner FROM locks WHERE repo_id = ?1 AND branch = ?2 AND path = ?3",
                    params![repo_id, branch, np],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| VcsError::Internal(e.into()))?;

            if let Some(holder) = existing {
                if !holder.eq_ignore_ascii_case(owner) {
                    return Err(VcsError::LockedByOther {
                        path: np.clone(),
                        owner: holder,
                    });
                }
            }
        }

        let mut acquired = Vec::with_capacity(norm_paths.len());
        for np in norm_paths {
            tx.execute(
                "INSERT OR REPLACE INTO locks (repo_id, branch, path, owner, locked_at_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![repo_id, branch, np, owner, now],
            )
            .map_err(|e| VcsError::Internal(e.into()))?;
            acquired.push(LockInfo {
                path: np,
                branch: branch.to_string(),
                owner: owner.to_string(),
                locked_at_unix: now,
            });
        }

        tx.commit().map_err(|e| VcsError::Internal(e.into()))?;
        Ok(acquired)
    }

    pub fn release_locks(
        &self,
        repo: &str,
        branch: &str,
        requester: &AuthUser,
        paths: &[String],
        force: bool,
    ) -> Result<Vec<String>, VcsError> {
        let branch = if branch.trim().is_empty() {
            "main"
        } else {
            branch.trim()
        };
        if force && !requester.is_admin {
            return Err(VcsError::Forbidden);
        }

        let mut norm_paths = Vec::with_capacity(paths.len());
        for p in paths {
            let Some(np) = normalize_rel_path(p) else {
                return Err(VcsError::BadRequest(format!("invalid path: {}", p)));
            };
            norm_paths.push(np);
        }

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(|e| VcsError::Internal(e.into()))?;
        let repo_id = Self::get_repo_id(&tx, repo)?;

        let mut released = Vec::new();
        for np in norm_paths {
            let holder: Option<String> = tx
                .query_row(
                    "SELECT owner FROM locks WHERE repo_id = ?1 AND branch = ?2 AND path = ?3",
                    params![repo_id, branch, np],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| VcsError::Internal(e.into()))?;

            if let Some(owner) = holder {
                if !force && !owner.eq_ignore_ascii_case(&requester.username) {
                    return Err(VcsError::LockedByOther {
                        path: np,
                        owner,
                    });
                }
                tx.execute(
                    "DELETE FROM locks WHERE repo_id = ?1 AND branch = ?2 AND path = ?3",
                    params![repo_id, branch, np],
                )
                .map_err(|e| VcsError::Internal(e.into()))?;
                released.push(np);
            }
        }

        tx.commit().map_err(|e| VcsError::Internal(e.into()))?;
        Ok(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_env(name: &str) -> (Database, BlobStore, std::path::PathBuf) {
        let dir = env::temp_dir().join(format!("chibivcs_test_{}_{}", name, random_hex(6)));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Database::open(dir.join("vcs.db")).unwrap();
        let blobs = BlobStore::new(dir.join("blobs")).unwrap();
        (db, blobs, dir)
    }

    #[test]
    fn test_auth_and_locks_and_auto_rebase() {
        let (db, blobs, dir) = temp_env("core");
        db.upsert_user("LogicCuteGuy", "secret1", true).unwrap();
        db.upsert_user("kamikenza", "secret2", false).unwrap();

        assert!(db.verify_login("LogicCuteGuy", "wrong").unwrap().is_none());
        let login = db.verify_login("LogicCuteGuy", "secret1").unwrap().unwrap();
        assert!(login.is_admin);
        let user1 = db.authenticate_token(&login.token).unwrap().unwrap();
        assert_eq!(user1.username, "LogicCuteGuy");

        let login2 = db.verify_login("kamikenza", "secret2").unwrap().unwrap();
        let user2 = db.authenticate_token(&login2.token).unwrap().unwrap();

        db.create_repo("vrc-world", "Test World", &user1.username)
            .unwrap();

        // Upload blobs
        let scene_bytes = b"unity scene v1";
        let scene_hash = sha256_hex(scene_bytes);
        blobs.put(&scene_hash, scene_bytes).unwrap();

        let script_a = b"class A {}";
        let script_a_hash = sha256_hex(script_a);
        blobs.put(&script_a_hash, script_a).unwrap();

        let script_b = b"class B {}";
        let script_b_hash = sha256_hex(script_b);
        blobs.put(&script_b_hash, script_b).unwrap();

        // Checkin cs:1
        let cs1 = db
            .checkin(
                "vrc-world",
                &user1.username,
                CheckinRequest {
                    branch: "main".into(),
                    parent_changeset_id: 0,
                    comment: "initial import".into(),
                    changes: vec![
                        FileChangeItem {
                            path: "Assets/Scenes/World.unity".into(),
                            kind: ChangeKind::Added,
                            sha256: scene_hash.clone(),
                            size: scene_bytes.len() as u64,
                        },
                        FileChangeItem {
                            path: "Assets/Scripts/A.cs".into(),
                            kind: ChangeKind::Added,
                            sha256: script_a_hash.clone(),
                            size: script_a.len() as u64,
                        },
                    ],
                },
                &blobs,
            )
            .unwrap();
        assert_eq!(cs1.changeset_id, 1);
        assert!(!cs1.rebased);

        // User2 locks World.unity
        db.acquire_locks(
            "vrc-world",
            "main",
            &user2.username,
            &["Assets/Scenes/World.unity".into()],
        )
        .unwrap();

        // User1 tries to lock or checkin World.unity -> rejected with LockedByOther
        let lock_err = db
            .acquire_locks(
                "vrc-world",
                "main",
                &user1.username,
                &["Assets/Scenes/World.unity".into()],
            )
            .unwrap_err();
        assert!(matches!(lock_err, VcsError::LockedByOther { .. }));

        let ci_err = db
            .checkin(
                "vrc-world",
                &user1.username,
                CheckinRequest {
                    branch: "main".into(),
                    parent_changeset_id: 1,
                    comment: "try edit locked scene".into(),
                    changes: vec![FileChangeItem {
                        path: "Assets/Scenes/World.unity".into(),
                        kind: ChangeKind::Modified,
                        sha256: scene_hash.clone(),
                        size: scene_bytes.len() as u64,
                    }],
                },
                &blobs,
            )
            .unwrap_err();
        assert!(matches!(ci_err, VcsError::LockedByOther { .. }));

        // Concurrent non-overlapping checkins based on parent=1:
        // User2 checks in World.unity (becomes cs:2 and auto-releases User2's lock)
        let cs2 = db
            .checkin(
                "vrc-world",
                &user2.username,
                CheckinRequest {
                    branch: "main".into(),
                    parent_changeset_id: 1,
                    comment: "update scene".into(),
                    changes: vec![FileChangeItem {
                        path: "Assets/Scenes/World.unity".into(),
                        kind: ChangeKind::Modified,
                        sha256: scene_hash.clone(),
                        size: scene_bytes.len() as u64,
                    }],
                },
                &blobs,
            )
            .unwrap();
        assert_eq!(cs2.changeset_id, 2);
        assert!(db.list_locks("vrc-world", Some("main")).unwrap().is_empty());

        // User1 checks in B.cs still referencing parent=1 -> auto-rebases onto cs:2!
        let cs3 = db
            .checkin(
                "vrc-world",
                &user1.username,
                CheckinRequest {
                    branch: "main".into(),
                    parent_changeset_id: 1,
                    comment: "add B.cs".into(),
                    changes: vec![FileChangeItem {
                        path: "Assets/Scripts/B.cs".into(),
                        kind: ChangeKind::Added,
                        sha256: script_b_hash.clone(),
                        size: script_b.len() as u64,
                    }],
                },
                &blobs,
            )
            .unwrap();
        assert_eq!(cs3.changeset_id, 3);
        assert!(cs3.rebased);

        // Overlapping stale parent=1 on World.unity -> returns NeedsUpdate
        let overlap_err = db
            .checkin(
                "vrc-world",
                &user1.username,
                CheckinRequest {
                    branch: "main".into(),
                    parent_changeset_id: 1,
                    comment: "stale edit".into(),
                    changes: vec![FileChangeItem {
                        path: "Assets/Scenes/World.unity".into(),
                        kind: ChangeKind::Modified,
                        sha256: scene_hash,
                        size: scene_bytes.len() as u64,
                    }],
                },
                &blobs,
            )
            .unwrap_err();
        assert!(matches!(
            overlap_err,
            VcsError::NeedsUpdate {
                head_changeset_id: 3
            }
        ));

        let tree = db.get_tree("vrc-world", -1, Some("main")).unwrap();
        assert_eq!(tree.changeset_id, 3);
        assert_eq!(tree.files.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
