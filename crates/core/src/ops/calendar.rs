//! Calendar ops: event writes and deletes.

use super::*;

impl Engine {
    pub(crate) async fn run_cal_op(&self, ctx: &SyncCtx, providers: &Providers, op: &OpRow) -> Result<()> {
        let cal = providers.calendar.as_ref().ok_or_else(|| anyhow!("account has no calendar provider"))?;
        let p = &op.payload;
        match op.kind.as_str() {
            "cal_put" => {
                let event_id = p["event_id"].as_str().context("event_id")?;
                let Some(ev) = self.store.get_event(event_id)? else { return Ok(()) };
                let Some(calendar) = self.store.get_calendar(&ev.calendar_id)? else { return Ok(()) };
                let remote = (!ev.remote_id.starts_with("local:")).then(|| ev.remote_id.clone());
                let out = cal
                    .apply(ctx, &CalendarOp::Put { calendar_remote_id: calendar.remote_id.clone(), event_remote_id: remote.clone(), etag: ev.etag.clone(), uid: ev.uid.clone(), ics: ev.ics.clone() })
                    .await?;
                let rid = out.event_remote_id.or(remote).unwrap_or_else(|| ev.remote_id.clone());
                self.store.set_event_remote(event_id, &rid, out.event_etag.as_deref())?;
            }
            "cal_delete" => {
                let calendar_remote = p["calendar_remote_id"].as_str().context("calendar_remote_id")?.to_string();
                let remote = p["remote_id"].as_str().context("remote_id")?.to_string();
                if !remote.starts_with("local:") {
                    let notify = p.get("notify").and_then(Value::as_bool).unwrap_or(true);
                    cal.apply(ctx, &CalendarOp::Delete { calendar_remote_id: calendar_remote, event_remote_id: remote, etag: p.get("etag").and_then(Value::as_str).map(String::from), notify })
                        .await?;
                }
            }
            other => return Err(anyhow!("unknown calendar op {other}")),
        }
        ctx.calendar_changed();
        self.mirror_wake.notify_one();
        Ok(())
    }
}
