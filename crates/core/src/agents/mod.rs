//! Agents: who they are, and what they propose.

mod proposals;


use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::engine::Engine;

impl Engine {
    /// A new agent and its token. Only the token's hash is stored.
    pub(crate) fn create_agent(&self, name: &str, policy: &AgentPolicy) -> Result<(Agent, String)> {
        let token = format!("stk_{}", crate::random_token(24));
        let agent = self.store.insert_agent(name, &Self::hash_token(&token), policy)?;
        self.store.audit("user", "agent_created", &json!({ "agent": name }))?;
        Ok((agent, token))
    }

    /// The built-in agent behind token-less proposal files. What it can see
    /// follows the AI folder settings; its rate limit is the default policy's.
    pub(crate) fn folder_agent(&self, mirror: &MirrorSettings) -> Result<Agent> {
        if !mirror.accept_proposals {
            anyhow::bail!("proposals from the AI folder are turned off (Settings → AI folder); connected agents propose over MCP or /api/agent/rpc");
        }
        let mut a = self.store.builtin_agent("ai-folder", "AI folder", &AgentPolicy::default())?;
        a.policy.account_ids = mirror.account_ids.clone();
        a.policy.since_days = mirror.since_days;
        a.policy.allow_attachments = mirror.include_attachments;
        a.policy.allow_calendar = mirror.include_calendar;
        a.policy.allow_contacts = mirror.include_contacts;
        let _ = self.store.touch_agent(&a.id);
        Ok(a)
    }
    pub fn hash_token(token: &str) -> String {
        blake3::hash(token.as_bytes()).to_hex().to_string()
    }

    pub fn authenticate_agent(&self, token: &str) -> Option<Agent> {
        let a = self.store.agent_by_token_hash(&Self::hash_token(token.trim())).ok().flatten()?;
        let _ = self.store.touch_agent(&a.id);
        Some(a)
    }
}
