//! Favorite senders (0053).

use super::*;

impl Store {
    /// Add or remove addresses from the favorites. Returns how many changed.
    pub fn set_favorite_senders(&self, emails: &[String], favorite: bool) -> Result<usize> {
        let emails: Vec<String> = emails.iter().map(|e| e.trim().to_lowercase()).collect();
        self.w(|c| {
            let mut n = 0;
            for e in &emails {
                n += if favorite {
                    c.execute("INSERT INTO favorite_senders (email, created_at) VALUES (?,?) ON CONFLICT(email) DO NOTHING", params![e, now_ms()])?
                } else {
                    c.execute("DELETE FROM favorite_senders WHERE email=?", [e])?
                };
            }
            Ok(n)
        })
    }

    /// Favorite addresses (lowercase), sorted.
    pub fn list_favorite_senders(&self) -> Result<Vec<String>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT email FROM favorite_senders ORDER BY email")?;
            let rows = s.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(rows)
        })
    }
}
