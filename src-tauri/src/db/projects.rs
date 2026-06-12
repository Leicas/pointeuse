use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;

// ── Data types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedProject {
    pub id: i64,
    pub name: String,
}

/// Trait to abstract over different project representations so that
/// `cache_projects` can accept `&[ProjectInfo]`, `&[CachedProject]`, etc.
pub trait ProjectLike {
    fn project_id(&self) -> i64;
    fn project_name_ref(&self) -> &str;
}

impl ProjectLike for CachedProject {
    fn project_id(&self) -> i64 { self.id }
    fn project_name_ref(&self) -> &str { &self.name }
}

// ── Project cache ────────────────────────────────────────────────────

/// Upsert a batch of projects into the local cache.
pub fn cache_projects<T: ProjectLike>(conn: &Connection, projects: &[T]) -> AppResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO cached_projects (id, name)
             VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name",
        )?;
        for p in projects {
            stmt.execute(params![p.project_id(), p.project_name_ref()])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Return all cached projects, ordered by name.
pub fn get_cached_projects(conn: &Connection) -> AppResult<Vec<CachedProject>> {
    let mut stmt =
        conn.prepare("SELECT id, name FROM cached_projects ORDER BY name ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok(CachedProject {
            id: row.get(0)?,
            name: row.get(1)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}
