//! Settings and accounts.

use std::sync::Arc;

use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::engine::{Caller, Engine};
use crate::error::{err, invalid, not_found, to_api};
use crate::provider::CredentialHandle;
use crate::secrets::{MemSecretStore, SecretStore, account_key};

impl Engine {
    pub(crate) fn save_settings(&self, s: Settings) -> Result<()> {
        let _moving = self.mirror_lock.lock().unwrap();
        let old = self.settings();
        let (old_root, new_root) = (crate::ai_folder::root(&old.mirror), crate::ai_folder::root(&s.mirror));
        if old_root != new_root {
            crate::ai_folder::relocate(&old_root, &new_root)?;
            self.store.audit("user", "ai_folder_moved", &json!({ "from": old_root, "to": new_root }))?;
        }
        self.store.set_setting("settings", &s)?;
        let mirror_changed = serde_json::to_string(&old.mirror)? != serde_json::to_string(&s.mirror)?;
        *self.settings.write().unwrap() = s.clone();
        self.alerts_wake.notify_one();
        if old.calendar_alerts != s.calendar_alerts {
            // Hosts reschedule (or take down) the alerts they gave the OS.
            self.emit(CoreEvent::AlertsChanged { due: vec![], closed: vec![] });
        }
        if mirror_changed {
            if !s.mirror.enabled {
                let _ = crate::ai_folder::remove_dir_all(&new_root, &new_root.join("mail"));
            }
            self.store.reset_mirrored()?;
            self.mirror_wake.notify_one();
        }
        Ok(())
    }
    pub(crate) async fn add_account(self: &Arc<Self>, config: AccountConfig, credential: Credential) -> Result<Account> {
        let email = config.email().to_lowercase();
        if self.store.list_accounts()?.iter().any(|a| a.email.to_lowercase() == email && a.kind == config.kind()) {
            return Err(err(ErrorCode::Conflict, format!("{email} is already added")));
        }
        self.test_account(&config, &credential).await?;
        let acc = self.store.insert_account(&config)?;
        self.secrets.set(&account_key(&acc.id), &credential)?;
        self.store.audit("user", "account_added", &json!({ "email": acc.email, "kind": acc.kind }))?;
        self.start_account(&acc);
        self.emit(CoreEvent::AccountsChanged);
        Ok(acc)
    }

    /// Finish an OAuth sign-in: a new account, or fresh credentials (and
    /// scopes, e.g. contacts) for one that is already here.
    pub(crate) async fn oauth_signed_in(self: &Arc<Self>, config: AccountConfig, credential: Credential) -> Result<()> {
        let email = config.email().to_lowercase();
        let existing = self.store.list_accounts()?.into_iter().find(|a| a.email.to_lowercase() == email && a.kind == config.kind());
        let Some(acc) = existing else {
            self.add_account(config, credential).await?;
            return Ok(());
        };
        self.stop_account(&acc.id);
        self.secrets.set(&account_key(&acc.id), &credential)?;
        self.store.delete_cursor(&acc.id, "contacts_scope_missing")?;
        let acc = self.store.get_account(&acc.id)?.ok_or_else(|| not_found("account"))?;
        self.start_account(&acc);
        self.store.audit("user", "account_reconnected", &json!({ "email": acc.email }))?;
        self.emit(CoreEvent::AccountsChanged);
        self.emit(CoreEvent::Notice { level: "info".into(), message: format!("Reconnected {}", acc.email) });
        Ok(())
    }

    pub(crate) async fn test_account(&self, config: &AccountConfig, credential: &Credential) -> Result<()> {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecretStore::default());
        let tmp = Account {
            id: "test".into(),
            kind: config.kind(),
            email: config.email().into(),
            display_name: config.display_name().into(),
            config: config.clone(),
            sync: SyncStatus::default(),
            color: String::new(),
            contacts_need_reconnect: false,
        };
        secrets.set(&account_key("test"), credential)?;
        let p = self.factory.create(&tmp, CredentialHandle::new("test".into(), secrets))?;
        if let Some(m) = &p.mail {
            m.test().await?;
        }
        Ok(())
    }

    /// Contacts first (their name and every address), then other senders.
    pub(crate) fn suggest_addresses(&self, prefix: &str) -> Result<Vec<Address>> {
        let mut out: Vec<Address> = vec![];
        if !prefix.is_empty() {
            let p = prefix.to_lowercase();
            for c in self.store.list_contacts(Some(prefix), 20)? {
                let name_hit = c.fields.name.to_lowercase().split_whitespace().any(|w| w.starts_with(&p)) || c.fields.name.to_lowercase().starts_with(&p);
                for e in &c.fields.emails {
                    if name_hit || e.email.to_lowercase().starts_with(&p) {
                        out.push(Address { name: Some(c.fields.name.clone()).filter(|n| !n.is_empty()), email: e.email.clone() });
                    }
                }
            }
        }
        for a in self.store.known_senders(prefix, 8)? {
            if !out.iter().any(|x| x.email.eq_ignore_ascii_case(&a.email)) {
                out.push(a);
            }
        }
        out.truncate(8);
        Ok(out)
    }

    /// The accounts this caller may see.
    pub(crate) fn list_accounts(&self, caller: &Caller) -> Result<Vec<Account>> {
        let allowed = self.allowed_accounts(caller);
        Ok(self.store.list_accounts()?.into_iter().filter(|a| allowed.as_ref().is_none_or(|ids| ids.contains(&a.id))).collect())
    }

    pub(crate) fn remove_account(&self, account_id: &str) -> Result<()> {
        self.stop_account(account_id);
        let acc = self.store.get_account(account_id)?.ok_or_else(|| not_found("account"))?;
        self.store.delete_account(account_id)?;
        // Contacts that only lived in this account's address books go with it.
        self.store.prune_orphan_remote_contacts()?;
        self.emit(CoreEvent::ContactsChanged);
        if let Err(e) = self.free_unused_blobs() {
            tracing::warn!(account = %acc.email, "deleting the account's stored mail: {e:#}");
        }
        let secret = self.secrets.delete(&account_key(account_id));
        self.store.audit(
            "user",
            "account_removed",
            &json!({ "email": acc.email, "credential_error": secret.as_ref().err().map(|e| format!("{e:#}")) }),
        )?;
        self.mirror_wake.notify_one();
        self.emit(CoreEvent::AccountsChanged);
        // The account is gone either way; say so if its password is still saved.
        secret.map_err(|e| err(ErrorCode::Internal, format!("{} was removed, but its saved password couldn't be deleted: {e:#}", acc.email)))
    }

    pub(crate) async fn update_account(self: &Arc<Self>, account_id: &str, config: AccountConfig, credential: Option<Credential>) -> Result<Account> {
        let acc = self.store.get_account(account_id)?.ok_or_else(|| not_found("account"))?;
        if acc.kind != config.kind() {
            return Err(invalid("account type can't change; add a new account instead"));
        }
        let cred = match credential {
            Some(c) => c,
            None => self.secrets.get(&account_key(account_id))?.ok_or_else(|| err(ErrorCode::Auth, "no stored credential"))?,
        };
        self.test_account(&config, &cred).await?;
        self.stop_account(account_id);
        self.store.update_account_config(account_id, &config)?;
        self.secrets.set(&account_key(account_id), &cred)?;
        let acc = self.store.get_account(account_id)?.ok_or_else(|| not_found("account"))?;
        self.start_account(&acc);
        self.store.audit("user", "account_updated", &json!({ "email": acc.email }))?;
        self.emit(CoreEvent::AccountsChanged);
        Ok(acc)
    }

    /// Start a browser sign-in; returns the URL to open. The account is added
    /// when the provider redirects back.
    pub(crate) fn start_oauth(self: &Arc<Self>, provider: ProviderKind) -> Result<String> {
        let weak = Arc::downgrade(self);
        let done: crate::provider::OAuthDone = Box::new(move |res| {
            let Some(engine) = weak.upgrade() else { return };
            tokio::spawn(async move {
                let r = match res {
                    Ok((config, credential)) => engine.oauth_signed_in(config, credential).await,
                    Err(e) => Err(e),
                };
                if let Err(e) = r {
                    engine.emit(CoreEvent::Notice { level: "error".into(), message: format!("Sign-in failed: {}", to_api(e).message) });
                }
            });
        });
        self.factory.oauth_start(provider, done)
    }

    pub(crate) fn sync_now(&self, account_id: Option<String>) -> Result<()> {
        let ids = match account_id {
            Some(a) => vec![a],
            None => self.store.list_accounts()?.into_iter().map(|a| a.id).collect(),
        };
        for a in ids {
            self.contacts_due.lock().unwrap().insert(a.clone());
            self.wake_account(&a);
        }
        Ok(())
    }
}
