//! Favorite senders: addresses whose mail the user wants to stand out. A
//! local list, never sent to a server. See docs/decisions/0053-favorite-senders.md.

use anyhow::Result;
use satchel_api::*;
use serde_json::json;

use crate::contacts::valid_address;
use crate::engine::Engine;
use crate::error::invalid;

impl Engine {
    pub(crate) fn set_favorite_senders(&self, emails: &[String], favorite: bool) -> Result<()> {
        let emails: Vec<String> = emails.iter().map(|e| e.trim().to_lowercase()).collect();
        if let Some(bad) = emails.iter().find(|e| !valid_address(e)) {
            return Err(invalid(format!("invalid address: {bad}")));
        }
        if self.store.set_favorite_senders(&emails, favorite)? > 0 {
            self.store.audit("user", if favorite { "favorite_added" } else { "favorite_removed" }, &json!({ "emails": emails }))?;
            self.emit(CoreEvent::FavoritesChanged);
        }
        Ok(())
    }
}
