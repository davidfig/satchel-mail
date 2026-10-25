//! The agenda and the calendar's .ics files.

use super::*;

impl Mirror<'_> {
    pub fn write_calendar(&self) -> Result<()> {
        let base = self.root().join("calendar");
        let from = Utc::now() - Duration::days(7);
        let to = Utc::now() + Duration::days(60);
        let occ = crate::calendar::occurrences(self.store, from, to, None)?;
        let cals = self.store.list_calendars(None)?;
        let mut md = String::from("# Agenda (past 7 days to next 60 days, UTC)\n\n");
        let mut last_day = String::new();
        for o in &occ {
            if !self.account_allowed(&cals.iter().find(|c| c.cal.id == o.event.calendar_id).map(|c| c.cal.account_id.clone()).unwrap_or_default()) {
                continue;
            }
            let day = o.start.format("%Y-%m-%d (%a)").to_string();
            if day != last_day {
                md.push_str(&format!("\n## {day}\n\n"));
                last_day = day;
            }
            let cal = cals.iter().find(|c| c.cal.id == o.event.calendar_id).map(|c| c.cal.name.clone()).unwrap_or_default();
            let time = if o.event.all_day { "all day".to_string() } else { format!("{}–{}", o.start.format("%H:%M"), o.end.format("%H:%M")) };
            md.push_str(&format!("- {time} **{}** [{cal}] event_id `{}`", o.event.title, o.event.id.clone().unwrap_or_default()));
            if !o.event.location.is_empty() {
                md.push_str(&format!(" @ {}", o.event.location));
            }
            if let Some(s) = o.event.my_status {
                md.push_str(&format!(" (you: {})", satchel_ical::partstat_str(s).to_lowercase()));
            }
            md.push('\n');
        }
        write_if_changed(&self.root(), &base.join("AGENDA.md"), md.as_bytes())?;
        for c in &cals {
            if !self.account_allowed(&c.cal.account_id) {
                continue;
            }
            let acc = self.store.get_account(&c.cal.account_id)?.map(|a| a.email).unwrap_or_default();
            let dir = base.join(safe_name(&acc)).join(safe_name(&c.cal.name));
            for e in self.store.events_in_range(from - Duration::days(365), to + Duration::days(365), Some(std::slice::from_ref(&c.cal.id)))? {
                write_if_changed(&self.root(), &dir.join(format!("{}.ics", safe_name(&e.uid))), e.ics.as_bytes())?;
            }
        }
        Ok(())
    }
}
