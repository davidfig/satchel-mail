//! What a caller may see and do: agent policy checks.


use anyhow::Result;
use chrono::{Duration, Utc};
use satchel_api::*;

use crate::engine::{Caller, Engine, Mode};
use crate::error::{err, forbidden, not_found};

impl Engine {
    pub(crate) fn allowed_accounts(&self, caller: &Caller) -> Option<Vec<String>> {
        match caller {
            Caller::User => None,
            Caller::Agent(a) => a.policy.account_ids.clone(),
        }
    }

    pub(crate) fn check_account(&self, caller: &Caller, account_id: &str) -> Result<()> {
        if let Some(ids) = self.allowed_accounts(caller)
            && !ids.iter().any(|x| x == account_id) {
                return Err(forbidden("account not shared with this agent"));
            }
        Ok(())
    }

    pub(crate) fn cutoff(&self, caller: &Caller) -> Option<chrono::DateTime<Utc>> {
        match caller {
            Caller::Agent(a) => a.policy.since_days.map(|d| Utc::now() - Duration::days(d as i64)),
            Caller::User => None,
        }
    }

    pub(crate) fn check_calendar_access(&self, caller: &Caller) -> Result<()> {
        if let Caller::Agent(a) = caller
            && !a.policy.allow_calendar {
                return Err(forbidden("calendar access is not enabled for this agent"));
            }
        Ok(())
    }

    pub(crate) fn check_message(&self, caller: &Caller, id: &str) -> Result<crate::store::StoredMessage> {
        let m = self.store.message_row(id)?.ok_or_else(|| not_found("message"))?;
        self.check_account(caller, &m.summary.account_id)?;
        if let Some(c) = self.cutoff(caller)
            && m.summary.date < c {
                return Err(forbidden("message is outside this agent's allowed date range"));
            }
        Ok(m)
    }

    pub(crate) fn require_user(&self, caller: &Caller) -> Result<()> {
        match caller {
            Caller::User => Ok(()),
            Caller::Agent(_) => Err(forbidden("requires the user")),
        }
    }

    pub(crate) fn require_full(&self) -> Result<()> {
        if self.mode != Mode::Full {
            return Err(err(ErrorCode::Conflict, "this process is read-only; open Satchel Mail to make changes"));
        }
        Ok(())
    }
}
