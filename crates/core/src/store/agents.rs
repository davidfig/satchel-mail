//! Agents and their proposals.

use super::*;

impl Store {
    pub fn insert_agent(&self, name: &str, token_hash: &str, policy: &AgentPolicy) -> Result<Agent> {
        let id = new_id();
        self.w(|c| {
            c.execute(
                "INSERT INTO agents (id, name, token_hash, created_at, policy_json) VALUES (?,?,?,?,?)",
                params![id, name, token_hash, now_ms(), to_json(policy)],
            )?;
            Ok(())
        })?;
        self.get_agent(&id)?.ok_or_else(|| anyhow!("agent vanished"))
    }

    fn agent_row(r: &Row) -> rusqlite::Result<Agent> {
        Ok(Agent {
            id: r.get("id")?,
            name: r.get("name")?,
            created_at: ms_to_dt(r.get("created_at")?),
            last_used: r.get::<_, Option<i64>>("last_used")?.map(ms_to_dt),
            policy: serde_json::from_str(&r.get::<_, String>("policy_json")?).unwrap_or_default(),
            revoked: r.get("revoked")?,
            builtin: r.get::<_, String>("token_hash")?.starts_with("builtin:"),
        })
    }

    pub fn list_agents(&self) -> Result<Vec<Agent>> {
        self.r(|c| {
            let mut s = c.prepare("SELECT * FROM agents ORDER BY created_at")?;
            let rows = s.query_map([], Self::agent_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM agents WHERE id=?", [id], Self::agent_row).optional()?))
    }

    pub fn agent_by_token_hash(&self, hash: &str) -> Result<Option<Agent>> {
        self.r(|c| Ok(c.query_row("SELECT * FROM agents WHERE token_hash=? AND revoked=0", [hash], Self::agent_row).optional()?))
    }

    /// A built-in agent with no token (e.g. the AI folder), created on first use.
    /// Its `token_hash` is a marker no real token hashes to.
    pub fn builtin_agent(&self, key: &str, name: &str, policy: &AgentPolicy) -> Result<Agent> {
        let marker = format!("builtin:{key}");
        let found = self.r(|c| Ok(c.query_row("SELECT * FROM agents WHERE token_hash=?", [&marker], Self::agent_row).optional()?))?;
        match found {
            Some(a) => Ok(a),
            None => self.insert_agent(name, &marker, policy),
        }
    }

    pub fn update_agent(&self, id: &str, name: Option<&str>, policy: Option<&AgentPolicy>) -> Result<()> {
        self.w(|c| {
            if let Some(n) = name {
                c.execute("UPDATE agents SET name=? WHERE id=?", params![n, id])?;
            }
            if let Some(p) = policy {
                c.execute("UPDATE agents SET policy_json=? WHERE id=?", params![to_json(p), id])?;
            }
            Ok(())
        })
    }

    pub fn touch_agent(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE agents SET last_used=? WHERE id=?", params![now_ms(), id])?;
            Ok(())
        })
    }

    pub fn revoke_agent(&self, id: &str) -> Result<()> {
        self.w(|c| {
            c.execute("UPDATE agents SET revoked=1 WHERE id=?", [id])?;
            Ok(())
        })
    }


    pub fn insert_proposal(&self, p: &Proposal, versions: &HashMap<String, i64>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "INSERT INTO proposals (id, agent_id, created_at, status, action_json, rationale, source_threads_json, warnings_json, summary, versions_json)
                 VALUES (?,?,?,?,?,?,?,?,?,?)",
                params![
                    p.id, p.agent_id, p.created_at.timestamp_millis(), proposal_status_str(p.status), to_json(&p.action), p.rationale,
                    to_json(&p.source_thread_ids), to_json(&p.warnings), p.summary, to_json(versions)
                ],
            )?;
            Ok(())
        })
    }

    fn proposal_row(r: &Row) -> rusqlite::Result<(Proposal, HashMap<String, i64>)> {
        let action: ProposalAction = serde_json::from_str(&r.get::<_, String>("action_json")?).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok((
            Proposal {
                id: r.get("id")?,
                agent_id: r.get("agent_id")?,
                agent_name: r.get::<_, Option<String>>("agent_name").ok().flatten().unwrap_or_default(),
                created_at: ms_to_dt(r.get("created_at")?),
                status: parse_proposal_status(&r.get::<_, String>("status")?),
                action,
                rationale: r.get("rationale")?,
                source_thread_ids: from_json(&r.get::<_, String>("source_threads_json")?),
                warnings: from_json(&r.get::<_, String>("warnings_json")?),
                review_note: r.get("review_note")?,
                decided_at: r.get::<_, Option<i64>>("decided_at")?.map(ms_to_dt),
                summary: r.get("summary")?,
            },
            from_json(&r.get::<_, String>("versions_json")?),
        ))
    }

    pub fn list_proposals(&self, status: Option<ProposalStatus>, agent_id: Option<&str>) -> Result<Vec<Proposal>> {
        self.r(|c| {
            let mut sql = String::from(
                "SELECT p.*, a.name AS agent_name FROM proposals p LEFT JOIN agents a ON a.id=p.agent_id WHERE 1=1",
            );
            let mut p: Vec<String> = vec![];
            if let Some(s) = status {
                sql.push_str(" AND p.status=?");
                p.push(proposal_status_str(s).into());
            }
            if let Some(a) = agent_id {
                sql.push_str(" AND p.agent_id=?");
                p.push(a.into());
            }
            sql.push_str(" ORDER BY p.created_at DESC LIMIT 500");
            let mut s = c.prepare(&sql)?;
            let rows = s
                .query_map(params_from_iter(p.iter()), Self::proposal_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(|x| x.0).collect())
        })
    }

    pub fn get_proposal(&self, id: &str) -> Result<Option<(Proposal, HashMap<String, i64>)>> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT p.*, a.name AS agent_name FROM proposals p LEFT JOIN agents a ON a.id=p.agent_id WHERE p.id=?",
                [id],
                Self::proposal_row,
            )
            .optional()?)
        })
    }

    pub fn set_proposal_status(&self, id: &str, status: ProposalStatus, note: Option<&str>, action: Option<&ProposalAction>) -> Result<()> {
        self.w(|c| {
            c.execute(
                "UPDATE proposals SET status=?, review_note=COALESCE(?, review_note), decided_at=?, action_json=COALESCE(?, action_json) WHERE id=?",
                params![proposal_status_str(status), note, now_ms(), action.map(to_json), id],
            )?;
            Ok(())
        })
    }

    pub fn proposals_since(&self, agent_id: &str, since: DateTime<Utc>) -> Result<u32> {
        self.r(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM proposals WHERE agent_id=? AND created_at>=?",
                params![agent_id, since.timestamp_millis()],
                |r| r.get(0),
            )?)
        })
    }
}
