//! What syncs: this computer's settings as items, and putting another
//! computer's items into effect here (0080).

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use satchel_api::*;
use serde_json::{Map, Value, json};

use crate::Engine;

/// Preferences that follow the person. The rest (the AI folder, disk use,
/// how much mail to fetch) are about this computer.
const SYNCED_SETTINGS: &[&str] = &[
    "remote_image_senders",
    "load_remote_images",
    "sync_ai_drafts_to_server",
    "theme",
    "poll_interval_secs",
    "notify_new_mail",
    "look_up_places",
    "calendar_alerts",
    "default_alert_minutes",
];

/// Every synced item as this computer has it now.
pub fn local(engine: &Engine) -> Result<BTreeMap<String, Value>> {
    let store = engine.store();
    let mut out = BTreeMap::new();

    let all = serde_json::to_value(engine.settings())?;
    let picked: Map<String, Value> =
        SYNCED_SETTINGS.iter().filter_map(|k| all.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
    out.insert("settings".into(), Value::Object(picked));

    let accounts = store.list_accounts()?;
    let email_of: HashMap<&str, String> = accounts.iter().map(|a| (a.id.as_str(), a.email.to_lowercase())).collect();
    for a in &accounts {
        out.insert(format!("account:{}", a.email.to_lowercase()), json!({ "config": a.config, "color": a.color }));
    }

    // Built-in agents (the AI folder) take their policy from this computer's
    // AI folder settings, so they don't sync.
    for a in store.list_agents()?.into_iter().filter(|a| !a.revoked && !a.builtin) {
        let mut policy = serde_json::to_value(&a.policy)?;
        if let Some(ids) = &a.policy.account_ids {
            let mut emails: Vec<String> = ids.iter().filter_map(|id| email_of.get(id.as_str()).cloned()).collect();
            emails.sort();
            policy["account_ids"] = json!(emails);
        }
        out.insert(format!("agent:{}", a.id), json!({ "name": a.name, "policy": policy }));
    }

    // The place is the folder's index in the sidebar, so it reads the same
    // on every computer whatever the stored numbers are.
    for (i, f) in store.list_smart_folders()?.into_iter().enumerate() {
        out.insert(format!("smart_folder:{}", f.id), json!({ "name": f.name, "query": f.query, "badge": f.badge, "position": i }));
    }
    for r in store.list_sender_rules()? {
        out.insert(format!("sender_rule:{}", r.email.to_lowercase()), json!(r.verdict));
    }
    for e in store.list_favorite_senders()? {
        out.insert(format!("favorite:{}", e.to_lowercase()), json!(true));
    }
    super::proposals::local(engine, &mut out)?;
    Ok(out)
}

/// Puts another computer's item into effect here. `v` is null when it was
/// removed there.
pub fn apply(engine: &Engine, key: &str, v: &Value) -> Result<super::proposals::Outcome> {
    let (kind, rest) = key.split_once(':').unwrap_or((key, ""));
    match kind {
        "settings" => {
            let Value::Object(theirs) = v else { return Ok(super::proposals::Outcome::Done) };
            let mut s = serde_json::to_value(engine.settings())?;
            for (k, val) in theirs {
                if SYNCED_SETTINGS.contains(&k.as_str()) {
                    s[k] = val.clone();
                }
            }
            engine.save_settings(serde_json::from_value(s)?)?;
        }
        // Offered in Settings → Sync, never added without signing in.
        "account" => {}
        "agent" => apply_agent(engine, rest, v)?,
        "smart_folder" => {
            if v.is_null() {
                engine.store().delete_smart_folder(rest)?;
            } else {
                let f = SmartFolder {
                    id: rest.into(),
                    name: str_of(v, "name"),
                    query: str_of(v, "query"),
                    badge: serde_json::from_value(v["badge"].clone()).unwrap_or_default(),
                    count: 0,
                };
                engine.store().put_smart_folder(&f, v["position"].as_i64())?;
            }
            engine.emit(CoreEvent::SmartFoldersChanged);
        }
        "sender_rule" => {
            let verdict: Option<SenderVerdict> = serde_json::from_value(v.clone()).unwrap_or(None);
            engine.set_sender_rule(rest, verdict)?;
        }
        "favorite" => {
            engine.set_favorite_senders(&[rest.to_string()], v == &json!(true))?;
        }
        "proposal" => return super::proposals::apply(engine, rest, v),
        _ => {} // from a newer Satchel
    }
    Ok(super::proposals::Outcome::Done)
}

fn str_of(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().to_string()
}

fn apply_agent(engine: &Engine, id: &str, v: &Value) -> Result<()> {
    let store = engine.store();
    let existing = store.get_agent(id)?;
    if existing.as_ref().is_some_and(|a| a.builtin) {
        return Ok(());
    }
    if v.is_null() {
        // Revoked on another computer: revoke here too.
        if let Some(a) = existing.filter(|a| !a.revoked) {
            store.revoke_agent(&a.id)?;
            store.audit("sync", "agent_revoked", &json!({ "agent": a.id }))?;
        }
        return Ok(());
    }
    let mut policy = v["policy"].clone();
    if let Some(emails) = policy["account_ids"].as_array() {
        let id_of: HashMap<String, String> =
            store.list_accounts()?.into_iter().map(|a| (a.email.to_lowercase(), a.id)).collect();
        let ids: Vec<&String> = emails.iter().filter_map(|e| e.as_str().and_then(|e| id_of.get(&e.to_lowercase()))).collect();
        policy["account_ids"] = json!(ids);
    }
    let policy: AgentPolicy = serde_json::from_value(policy)?;
    let name = str_of(v, "name");
    match existing {
        // Revoking is final: a revoked agent isn't brought back.
        Some(a) if a.revoked => {}
        Some(a) => store.update_agent(&a.id, (!name.is_empty()).then_some(name.as_str()), Some(&policy))?,
        None => store.insert_synced_agent(id, &name, &policy)?,
    }
    store.audit("sync", "agent_policy_synced", &json!({ "agent": id }))?;
    Ok(())
}

/// An agent changed here can't name an account this computer doesn't have,
/// but other computers' accounts in its access must not be dropped by that:
/// they are carried over from the agent's last synced value.
pub fn keep_absent_accounts(new: &mut Value, prev: &Value, here: &std::collections::BTreeSet<String>) {
    let (Some(now), Some(before)) = (new["policy"]["account_ids"].as_array(), prev["policy"]["account_ids"].as_array()) else { return };
    let mut all: Vec<String> = now.iter().filter_map(|e| e.as_str().map(String::from)).collect();
    for e in before.iter().filter_map(|e| e.as_str()) {
        if !here.contains(e) && !all.iter().any(|x| x == e) {
            all.push(e.to_string());
        }
    }
    all.sort();
    new["policy"]["account_ids"] = json!(all);
}

/// Whether an agent's synced value gives access to any of `emails`.
pub fn names_any(v: &Value, emails: &std::collections::BTreeSet<String>) -> bool {
    v["policy"]["account_ids"].as_array().is_some_and(|a| a.iter().filter_map(|e| e.as_str()).any(|e| emails.contains(e)))
}
