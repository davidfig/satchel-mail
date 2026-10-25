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

    // The user's decision and note are written back to the file.
    engine.handle(&Caller::User, Command::RejectProposal { proposal_id: id, note: Some("Not \"yet\"".into()) }).await.unwrap();
    let done = std::fs::read_to_string(dir.path().join("mirror/proposals/processed/archive.md")).unwrap();
    assert_eq!(field(&done, "status"), Some("rejected"), "{done}");
    assert_eq!(field(&done, "note"), Some(r#""Not \"yet\"""#));

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
async fn folder_proposals_can_be_turned_off_but_tokens_still_work() {
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
    let done = drop_proposal(&engine, &dir, "flag2.md", &format!("---\naction: flag\nmessages: [{invoice}]\nagent_token: {token}\n---\n")).await;
    assert_eq!(field(&done, "status"), Some("pending"), "{done}");
    assert!(!done.contains(token), "the token is stripped");
    let p: Proposal = serde_json::from_value(engine.handle(&Caller::User, Command::GetProposal { proposal_id: field(&done, "proposal").unwrap().into() }).await.unwrap()).unwrap();
    assert_eq!(p.agent_name, "Codex");
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
