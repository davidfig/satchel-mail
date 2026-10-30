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
            let cal = cals.iter().find(|c| c.cal.id == o.event.calendar_id).map(|c| one_line(&c.cal.name)).unwrap_or_default();
            let time = if o.event.all_day { "all day".to_string() } else { format!("{}–{}", o.start.format("%H:%M"), o.end.format("%H:%M")) };
            md.push_str(&format!("- {time} **{}** [{cal}] event_id `{}`", one_line(&o.event.title).replace('*', "\\*"), one_line(&o.event.id.clone().unwrap_or_default()).replace('`', "")));
            if !o.event.location.is_empty() {
                md.push_str(&format!(" @ {}", one_line(&o.event.location)));
            }
            if let Some(s) = o.event.my_status {
                md.push_str(&format!(" (you: {})", satchel_ical::partstat_str(s).to_lowercase()));
            }
            md.push('\n');
        }
        if let Err(e) = write_if_changed(&self.root(), &base.join("AGENDA.md"), md.as_bytes()) {
            tracing::warn!("AI folder: AGENDA.md: {e:#}");
        }
        // One directory per account, one per calendar in it, one file per
        // event. Names that would clash (on a case-insensitive disk too) get a
        // hash; each event's file always has one, since UIDs differ in case.
        let mut account_dirs: HashMap<String, Names> = HashMap::new();
        let mut top = Names::reserving(&["AGENDA.md"]);
        let mut acc_names: HashMap<String, String> = HashMap::new();
        for c in &cals {
            if !self.account_allowed(&c.cal.account_id) {
                continue;
            }
            let acc_dir = match acc_names.get(&c.cal.account_id) {
                Some(d) => d.clone(),
                None => {
                    let acc = self.store.get_account(&c.cal.account_id)?.map(|a| a.email).unwrap_or_default();
                    let d = top.take(&safe_name(&acc), "", &c.cal.account_id);
                    acc_names.insert(c.cal.account_id.clone(), d.clone());
                    d
                }
            };
            let cal_dir = account_dirs.entry(acc_dir.clone()).or_insert_with(|| Names::reserving(&[])).take(&safe_name(&c.cal.name), "", &c.cal.id);
            let dir = base.join(&acc_dir).join(cal_dir);
            let mut written = HashSet::new();
            for e in self.store.events_in_range(from - Duration::days(365), to + Duration::days(365), Some(std::slice::from_ref(&c.cal.id)))? {
                let file = format!("{}-{}.ics", safe_name(&e.uid), short_hash(&e.uid));
                match write_if_changed(&self.root(), &dir.join(&file), e.ics.as_bytes()) {
                    Ok(()) => {
                        written.insert(file);
                    }
                    Err(err) => tracing::warn!("AI folder: event {}: {err:#}", e.uid),
                }
            }
            // Events gone from the range (and files named the old way) go.
            for p in list_files(&self.root(), &dir).unwrap_or_default() {
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if name.ends_with(".ics") && !written.contains(&name) {
                    let _ = remove_file(&self.root(), &p);
                }
            }
        }
        Ok(())
    }
}
