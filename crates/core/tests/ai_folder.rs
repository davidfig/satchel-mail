//! Proposal files in the AI folder, and symlinks an agent might plant there.

mod support;

use support::mail::*;

#[tokio::test(flavor = "multi_thread")]
async fn markdown_proposal_files_need_no_token_and_report_back() {
    let (engine, _fake, account, dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let invoice_thread = engine.store().thread_of(&invoice).unwrap().unwrap();
    let kickoff = engine.store().message_id_by_remote(&account, "r1").unwrap().unwrap();
    let kickoff_thread = engine.store().thread_of(&kickoff).unwrap().unwrap();

    // A token-less archive comes from the built-in AI folder agent.
    let done = drop_proposal(&engine, &dir, "archive.md", &format!("---\naction: archive\nthreads: [{invoice_thread}]\nwhy: Paid already\nagent: Claude\n---\n")).await;
    assert_eq!(field(&done, "status"), Some("pending"), "{done}");
    let id = field(&done, "proposal").unwrap().to_string();
    let p: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: id.clone() }).await.unwrap()).unwrap();
    assert_eq!(p.agent_name, "AI folder");
    assert_eq!(p.rationale, "[Claude] Paid already");
    assert!(matches!(&p.action, ProposalAction::Archive { message_ids } if message_ids == &vec![invoice.clone()]));
    let agents: Vec<Agent> = serde_json::from_value(engine.handle(&Caller::User, Command::ListAgents).await.unwrap()).unwrap();
    assert!(agents.iter().any(|a| a.builtin && a.name == "AI folder"));

    // The user's decision is written back to the file, but not their note:
    // every agent can write in processed/, so a note there proves nothing.
    engine.handle(&Caller::User, Command::RejectProposal { proposal_id: id, note: Some("Not \"yet\"".into()) }).await.unwrap();
    let done = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/archive.md")).unwrap();
    assert_eq!(field(&done, "status"), Some("rejected"), "{done}");
    assert_eq!(field(&done, "note"), None, "{done}");
    assert!(!done.contains("Not"), "{done}");

    // A reply goes to the sender of the thread's last message, with the body as text.
    let done = drop_proposal(&engine, &dir, "reply.md", &format!("---\naction: reply\nthread: {kickoff_thread}\nwhy: Confirm\n---\nMonday works.\n")).await;
    let id = field(&done, "proposal").unwrap().to_string();
    let p: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: id.clone() }).await.unwrap()).unwrap();
    let ProposalAction::Draft { draft } = &p.action else { panic!("expected a draft") };
    assert_eq!(draft.to, vec![Address { name: Some("Carol".into()), email: "carol@x.test".into() }]);
    assert_eq!(draft.subject, "Re: Kickoff");
    assert_eq!(draft.body_text, "Monday works.");
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    engine.handle(&Caller::User, Command::ApproveProposal { proposal_id: id, edited: None }).await.unwrap();
    let done = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/reply.md")).unwrap();
    assert_eq!(field(&done, "status"), Some("approved"));

    // Mistakes come back as errors, under a free name if the old one is taken.
    let done = drop_proposal(&engine, &dir, "oops.md", "---\naction: shred\n---\n").await;
    assert_eq!(field(&done, "status"), Some("error"));
    assert!(field(&done, "error").unwrap().contains("unknown action"));
    std::fs::write(dir.path().join("mirror/proposals/inbox/oops.md"), "no header").unwrap();
    engine.ingest_dropped_proposals().await;
    let again = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/oops-2.md")).unwrap();
    assert!(again.contains("no header"));
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_proposals_can_be_turned_off_and_tokens_are_refused() {
    let (engine, _fake, account, dir) = setup().await;
    let invoice = engine.store().message_id_by_remote(&account, "r3").unwrap().unwrap();
    let mut s = engine.settings();
    s.mirror.accept_proposals = false;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();

    let done = drop_proposal(&engine, &dir, "flag.md", &format!("---\naction: flag\nmessages: [{invoice}]\n---\n")).await;
    assert_eq!(field(&done, "status"), Some("error"));
    assert!(field(&done, "error").unwrap().contains("turned off"));

    let v = engine.handle(&Caller::User, Command::CreateAgent { name: "Codex".into(), policy: AgentPolicy::default() }).await.unwrap();
    let token = v["token"].as_str().unwrap();
    // Anything with folder access could read a token from a file, so one
    // is refused (not ignored), with a message saying where tokens go.
    for on in [false, true] {
        let mut s = engine.settings();
        s.mirror.accept_proposals = on;
        engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
        let name = format!("flag-{on}.md");
        let done = drop_proposal(&engine, &dir, &name, &format!("---\naction: flag\nmessages: [{invoice}]\nagent_token: {token}\n---\n")).await;
        assert_eq!(field(&done, "status"), Some("error"), "{done}");
        assert!(field(&done, "error").unwrap().contains("MCP"), "{done}");
        assert!(!done.contains(token), "the token is stripped");
        let name = format!("flag-{on}.json");
        let json = serde_json::json!({ "agent_token": token, "action": { "type": "flag", "message_ids": [invoice], "flagged": true } });
        let done = drop_proposal(&engine, &dir, &name, &json.to_string()).await;
        assert!(done.contains("aren't accepted"), "{done}");
    }
    let mine: Vec<Proposal> = serde_json::from_value(engine.handle(&Caller::User, Command::ListProposals { status: None }).await.unwrap()).unwrap();
    assert!(mine.is_empty(), "{mine:?}");
}


#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn symlinks_in_the_ai_folder_are_never_followed() {
    let (engine, _fake, _account, dir) = setup().await;
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("id_ed25519");
    std::fs::write(&secret, "PRIVATE KEY").unwrap();
    let inbox = dir.path().join("mirror/proposals/inbox");
    let processed = dir.path().join("mirror/proposals/processed");
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::create_dir_all(&processed).unwrap();
    // Read: a link in the inbox to a file outside.
    std::os::unix::fs::symlink(&secret, inbox.join("x.json")).unwrap();
    // Write: a dangling link where the processed copy would go.
    let planted = outside.path().join("evil.plist");
    std::os::unix::fs::symlink(&planted, processed.join("p.md")).unwrap();
    std::fs::write(inbox.join("p.md"), "<plist>not a proposal</plist>").unwrap();
    engine.ingest_dropped_proposals().await;
    for entry in std::fs::read_dir(&processed).unwrap().flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        assert!(!text.contains("PRIVATE KEY"), "secret copied into {:?}", entry.path());
    }
    assert!(!planted.exists(), "wrote through a planted symlink");
    assert_eq!(std::fs::read_to_string(&secret).unwrap(), "PRIVATE KEY");
}

#[tokio::test(flavor = "multi_thread")]
async fn smart_folders_are_listed_for_agents() {
    let (engine, _fake, _account, dir) = setup().await;
    let folder = SmartFolder { name: "Vendors".into(), query: "from:bill".into(), ..Default::default() };
    engine.handle(&Caller::User, Command::SaveSmartFolder { folder }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    let md = std::fs::read_to_string(dir.path().join("mirror/smart-folders.md")).unwrap();
    assert!(md.contains("## Vendors") && md.contains("Query: `from:bill`"), "{md}");
    assert!(md.contains("| Invoice |") && !md.contains("Kickoff"), "{md}");

    // The link is relative to the folder's root and leads to the thread.
    let link = md.split("](").nth(1).and_then(|l| l.split(')').next()).expect("a link").to_string();
    let file = dir.path().join("mirror").join(&link);
    assert!(file.is_file(), "{link}");

    // A thread that has gone from disk is listed without a dead link, once
    // the folder is next written whole (every ten minutes).
    std::fs::remove_file(&file).unwrap();
    engine.refresh_mirror();
    engine.mirror_pass(true).unwrap();
    let md = std::fs::read_to_string(dir.path().join("mirror/smart-folders.md")).unwrap();
    assert!(!md.contains(&link) && md.contains("(not written yet)"), "{md}");

    // A folder limited to some accounts doesn't list them: their queries can
    // describe any account's mail.
    let mut s = engine.settings();
    s.mirror.account_ids = Some(vec!["another".into()]);
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    assert!(!dir.path().join("mirror/smart-folders.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn changing_the_location_moves_the_folder() {
    let (engine, _fake, _account, dir) = setup().await;
    engine.mirror_pass(true).unwrap();
    let old = dir.path().join("mirror");
    std::fs::write(old.join("proposals/inbox/later.md"), "---\naction: archive\n---\n").unwrap();
    std::fs::write(old.join("agent-notes.md"), "mine").unwrap();

    // A folder that already has files is refused, and nothing moves.
    let full = dir.path().join("Documents");
    std::fs::create_dir_all(&full).unwrap();
    std::fs::write(full.join("taxes.pdf"), "x").unwrap();
    let mut s = engine.settings();
    s.mirror.root = Some(full.display().to_string());
    let err = engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap_err();
    assert!(err.message.contains("already has files"), "{err:?}");
    assert!(old.join("agent-notes.md").exists());

    let new = dir.path().join("elsewhere/Satchel");
    let mut s = engine.settings();
    s.mirror.root = Some(new.display().to_string());
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(new.join("agent-notes.md")).unwrap(), "mine");
    assert!(new.join("proposals/inbox/later.md").exists());
    assert!(new.join("mail").is_dir());
}

#[tokio::test(flavor = "multi_thread")]
async fn indexes_link_only_threads_on_disk_and_say_how_fresh_they_are() {
    let (engine, _fake, _account, dir) = setup().await;
    engine.mirror_pass(true).unwrap();
    let base = dir.path().join("mirror/mail/me@x.test");
    let read = |p: &str| std::fs::read_to_string(base.join(p)).unwrap();
    let rows = |jsonl: &str| jsonl.lines().map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).collect::<Vec<_>>();

    let index = rows(&read("index.jsonl"));
    assert!(!index.is_empty());
    for row in &index {
        let path = row["path"].as_str().expect("every thread is written");
        assert!(base.join(path).is_file(), "{path}");
        assert!(read("INDEX.md").contains(&format!("({path})")), "{path}");
    }
    let status: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("mirror/sync.json")).unwrap()).unwrap();
    assert_eq!(status["complete"], true, "{status}");
    assert_eq!(status["accounts"][0]["account"], "me@x.test", "{status}");

    // Empty mailboxes get no file.
    for e in std::fs::read_dir(&base).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.ends_with(".md") && name != "INDEX.md" {
            assert!(std::fs::read_to_string(e.path()).unwrap().lines().count() > 4, "{name} is empty");
        }
    }

    // A thread folder deleted behind Satchel's back is unlinked when the
    // folder is next written whole, then written again.
    let gone = index[0]["path"].as_str().unwrap().trim_end_matches("/thread.md").to_string();
    std::fs::remove_dir_all(base.join(&gone)).unwrap();
    engine.refresh_mirror();
    engine.mirror_pass(true).unwrap();
    assert!(!read("INDEX.md").contains(&format!("({gone}/thread.md)")));
    assert!(read("INDEX.md").contains("(not written yet)"));
    assert!(rows(&read("index.jsonl"))[0]["path"].is_null());
    let status = std::fs::read_to_string(dir.path().join("mirror/sync.json")).unwrap();
    assert!(status.contains("\"complete\": false"), "{status}");

    engine.mirror_pass(true).unwrap();
    assert!(base.join(&gone).join("thread.md").is_file());
    assert!(read("INDEX.md").contains(&format!("({gone}/thread.md)")));

    // A folder for an account Satchel doesn't have (removed, or written by
    // another data directory) goes.
    let stale = dir.path().join("mirror/mail/old@x.test");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("INDEX.md"), "# old").unwrap();
    engine.refresh_mirror();
    engine.mirror_pass(true).unwrap();
    assert!(!stale.exists());
    assert!(base.join("INDEX.md").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_folders_that_go_missing_are_found_and_written_again() {
    let (engine, _fake, account, dir) = setup().await;
    engine.mirror_pass(true).unwrap();
    let root = dir.path().join("mirror");
    let id = engine.store().message_id_by_remote(&account, "r1").unwrap().unwrap();
    let thread = engine.store().thread_of(&id).unwrap().unwrap();
    let folder = root.join(engine.store().mirror_path(&thread).unwrap().unwrap());
    assert!(folder.join("thread.md").is_file());
    let status = || std::fs::read_to_string(root.join("sync.json")).unwrap();
    assert!(status().contains("\"complete\": true"), "{}", status());

    // Gone behind Satchel's back. Without the indexes running, nothing
    // notices until the next check.
    std::fs::remove_dir_all(&folder).unwrap();
    engine.mirror_pass(false).unwrap();
    assert!(!folder.exists());

    // The check covers every thread, not just the ones the indexes list.
    engine.recheck_mirror();
    assert_eq!(engine.mirror_pass(false).unwrap(), 1);
    assert!(folder.join("thread.md").is_file());
    assert!(status().contains("\"complete\": true"), "{}", status());
}

#[tokio::test(flavor = "multi_thread")]
async fn history_lists_every_message_by_month() {
    let (engine, fake, account, dir) = setup().await;
    // Older than the folder's year of threads, with an attachment.
    let old = (chrono::Utc::now() - chrono::Duration::days(800)).to_rfc2822();
    let raw = format!(
        "From: Dana <dana@x.test>\r\nTo: me@x.test\r\nSubject: Old deck\r\nMessage-ID: <old@x.test>\r\nDate: {old}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSlides: https://x.test/d?utm_source=mail\r\n--b\r\nContent-Type: application/vnd.ms-powerpoint; name=\"Q3.ppt\"\r\nContent-Disposition: attachment; filename=\"Q3.ppt\"\r\nContent-Transfer-Encoding: base64\r\n\r\nQUFB\r\n--b--\r\n"
    );
    deliver(&fake, "old", raw.into_bytes(), "INBOX");
    engine.handle(&Caller::User, Command::SyncNow { account_id: None }).await.unwrap();
    wait_for(|| engine.store().message_id_by_remote(&account, "old").ok().flatten().is_some()).await;
    engine.mirror_pass(true).unwrap();

    let history = dir.path().join("mirror/mail/me@x.test/history");
    let rows = || -> Vec<serde_json::Value> {
        let mut files: Vec<_> = std::fs::read_dir(&history).unwrap().flatten().map(|e| e.path()).collect();
        files.sort();
        files.iter().flat_map(|f| std::fs::read_to_string(f).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect::<Vec<_>>()).collect()
    };
    let all = rows();
    assert_eq!(all.len(), 4, "every message, old ones too");
    let old_row = all.iter().find(|r| r["subject"] == "Old deck").unwrap();
    assert_eq!(old_row["from"]["email"], "dana@x.test");
    assert_eq!(old_row["from_me"], false);
    assert_eq!(old_row["attachments"][0]["name"], "Q3.ppt");
    assert_eq!(old_row["mailboxes"][0], "INBOX");
    assert!(!old_row["snippet"].as_str().unwrap().contains("utm_"), "{old_row}");
    let month = chrono::DateTime::parse_from_rfc3339(old_row["date"].as_str().unwrap()).unwrap().format("%Y-%m").to_string();
    assert!(history.join(format!("{month}.jsonl")).is_file());
    // No thread.md for it: it's outside the year the folder writes.
    let thread = old_row["thread_id"].as_str().unwrap();
    assert!(engine.store().mirror_path(thread).unwrap().is_none());

    // A change shows on the next pass.
    let kickoff = engine.store().message_id_by_remote(&account, "r1").unwrap().unwrap();
    engine.handle(&Caller::User, Command::SetFlags { message_ids: vec![kickoff.clone()], seen: Some(true), flagged: Some(true) }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    let row = rows().into_iter().find(|r| r["message_id"] == kickoff.as_str()).unwrap();
    assert_eq!((row["unread"].clone(), row["flagged"].clone()), (false.into(), true.into()));

    // Limited to the folder's dates, the old month goes.
    let mut s: Settings = serde_json::from_value(engine.handle(&Caller::User, Command::GetSettings).await.unwrap()).unwrap();
    s.mirror.history_all = false;
    engine.handle(&Caller::User, Command::UpdateSettings { settings: s }).await.unwrap();
    engine.mirror_pass(true).unwrap();
    assert!(!history.join(format!("{month}.jsonl")).exists());
    assert_eq!(rows().len(), 3);
}
