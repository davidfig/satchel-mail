//! `satchel` — headless host for the Satchel Mail core.
//!
//!   satchel serve            run sync + the web UI/API on localhost
//!   satchel mcp              MCP server over stdio for AI agents
//!   satchel accounts ...     list/add accounts
//!   satchel agents ...       create/list agent tokens
//!   satchel call '<json>'    run one API command (debugging)

mod server;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use satchel_api::*;
use satchel_core::secrets::{FileSecretStore, SecretStore};
use satchel_core::{Caller, Engine, EngineOptions, Mode};

#[derive(Parser)]
#[command(name = "satchel", version, about = "Satchel Mail headless host")]
struct Cli {
    /// Data directory (default: platform app-data dir, or $SATCHEL_DATA_DIR).
    #[arg(long, global = true, env = "SATCHEL_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Where credentials live: "keychain" (default) or "file" (data_dir/secrets.json, for CI/headless).
    #[arg(long, global = true, env = "SATCHEL_SECRETS", default_value = "keychain")]
    secrets: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run sync and serve the web UI + API on 127.0.0.1.
    Serve {
        #[arg(long, default_value_t = 7781)]
        port: u16,
        /// Directory with the built UI (ui/dist).
        #[arg(long)]
        ui: Option<PathBuf>,
    },
    /// MCP server over stdio. Requires an agent token.
    Mcp {
        #[arg(long, env = "SATCHEL_AGENT_TOKEN")]
        token: String,
    },
    /// Run one sync pass for every account, then exit.
    Sync {
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
    #[command(subcommand)]
    Accounts(AccountsCmd),
    #[command(subcommand)]
    Agents(AgentsCmd),
    /// Run one API command as the user, e.g. '{"cmd":"list_accounts"}'.
    Call { json: String },
}

#[derive(Subcommand)]
enum AccountsCmd {
    List,
    /// Add an IMAP/SMTP account (password from $SATCHEL_PASSWORD or --password).
    AddImap {
        #[arg(long)]
        email: String,
        #[arg(long, default_value = "")]
        name: String,
        #[arg(long)]
        username: Option<String>,
        #[arg(long, env = "SATCHEL_PASSWORD")]
        password: String,
        #[arg(long)]
        imap_host: String,
        #[arg(long, default_value_t = 993)]
        imap_port: u16,
        #[arg(long, default_value = "tls")]
        imap_security: String,
        #[arg(long)]
        smtp_host: Option<String>,
        #[arg(long, default_value_t = 465)]
        smtp_port: u16,
        #[arg(long, default_value = "tls")]
        smtp_security: String,
        #[arg(long)]
        caldav_url: Option<String>,
        /// CardDAV URL for contacts (default: discovered from the CalDAV server).
        #[arg(long)]
        carddav_url: Option<String>,
        /// Accept self-signed certificates (local test servers only).
        #[arg(long)]
        insecure: bool,
    },
    /// Add a JMAP account (e.g. Fastmail).
    AddJmap {
        #[arg(long)]
        email: String,
        #[arg(long, default_value = "")]
        name: String,
        #[arg(long)]
        username: Option<String>,
        #[arg(long, env = "SATCHEL_PASSWORD")]
        password: String,
        #[arg(long)]
        session_url: String,
        #[arg(long)]
        caldav_url: Option<String>,
        #[arg(long)]
        carddav_url: Option<String>,
        #[arg(long)]
        insecure: bool,
    },
    Remove { account_id: String },
}

#[derive(Subcommand)]
enum AgentsCmd {
    List,
    /// Create an agent and print its token (shown once).
    Create {
        name: String,
        /// Restrict to these account ids (comma-separated).
        #[arg(long)]
        accounts: Option<String>,
        #[arg(long)]
        since_days: Option<u32>,
        #[arg(long)]
        allow_attachments: bool,
    },
    Revoke { agent_id: String },
}

fn security(s: &str) -> Result<Security> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "tls" | "ssl" => Security::Tls,
        "starttls" => Security::StartTls,
        "none" | "plain" => Security::None,
        _ => bail!("security must be tls, starttls or none"),
    })
}

fn secrets_store(kind: &str, data_dir: &std::path::Path) -> Result<Arc<dyn SecretStore>> {
    match kind {
        "file" => Ok(Arc::new(FileSecretStore::new(data_dir.join("secrets.json"))?)),
        "keychain" => Ok(Arc::new(satchel_core::secrets::KeychainSecretStore::new("app.satchel.mail"))),
        other => bail!("unknown secrets store {other}"),
    }
}

/// Only one process may own sync for a data directory.
pub struct EngineLock(#[allow(dead_code)] std::fs::File);

pub fn try_lock(data_dir: &std::path::Path) -> Result<Option<EngineLock>> {
    std::fs::create_dir_all(data_dir)?;
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("engine.lock"))?;
    match f.try_lock() {
        Ok(()) => Ok(Some(EngineLock(f))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

fn open(cli: &Cli, mode: Mode, workers: bool) -> Result<(Arc<Engine>, Option<EngineLock>)> {
    let data_dir = cli.data_dir.clone().unwrap_or_else(satchel_core::default_data_dir);
    let (mode, lock) = if mode == Mode::Full {
        match try_lock(&data_dir)? {
            Some(l) => (Mode::Full, Some(l)),
            None => bail!("another Satchel Mail process (the app or `satchel serve`) is already running for {}", data_dir.display()),
        }
    } else {
        (mode, None)
    };
    let engine = Engine::open(EngineOptions {
        secrets: secrets_store(&cli.secrets, &data_dir)?,
        data_dir,
        mode,
        factory: Arc::new(satchel_providers::Factory::new()),
        start_workers: workers,
    })?;
    Ok((engine, lock))
}

/// Use the running app's engine if there is one (read/propose only), else own it.
fn open_for_call(cli: &Cli) -> Result<(Arc<Engine>, Option<EngineLock>)> {
    let data_dir = cli.data_dir.clone().unwrap_or_else(satchel_core::default_data_dir);
    match try_lock(&data_dir)? {
        Some(lock) => {
            drop(lock);
            open(cli, Mode::Full, false)
        }
        None => open(cli, Mode::Agent, false),
    }
}

async fn user(engine: &Arc<Engine>, cmd: Command) -> Result<serde_json::Value> {
    engine.handle(&Caller::User, cmd).await.map_err(|e| anyhow::anyhow!("{}", e.message))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let is_mcp = matches!(cli.cmd, Cmd::Mcp { .. });
    // MCP uses stdout for protocol messages: logs must go to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| if is_mcp { "warn".into() } else { "info,tantivy=warn".into() }))
        .with_writer(std::io::stderr)
        .init();

    match &cli.cmd {
        Cmd::Serve { port, ui } => {
            let (engine, _lock) = open(&cli, Mode::Full, true)?;
            let ui = ui.clone().or_else(default_ui_dir);
            server::serve(engine, *port, ui).await?;
        }
        Cmd::Mcp { token } => {
            let (engine, _lock) = open(&cli, Mode::Agent, true)?;
            let agent = engine.authenticate_agent(token).context("invalid or revoked agent token")?;
            satchel_mcp::SatchelMcp::new(engine, agent).serve_stdio().await?;
        }
        Cmd::Sync { timeout_secs } => {
            let (engine, _lock) = open(&cli, Mode::Full, true)?;
            let start = chrono::Utc::now();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(*timeout_secs);
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let accounts = engine.store().list_accounts()?;
                let done = accounts.iter().all(|a| a.sync.last_success.is_some_and(|t| t > start) || a.sync.state == SyncState::Error);
                if done && engine.store().unindexed(1)?.is_empty() {
                    for a in &accounts {
                        println!("{}\t{:?}\t{}", a.email, a.sync.state, a.sync.error.clone().unwrap_or_default());
                    }
                    break;
                }
                if std::time::Instant::now() > deadline {
                    bail!("sync timed out");
                }
            }
            engine.mirror_pass(true)?;
        }
        Cmd::Accounts(a) => {
            let (engine, _lock) = open_for_call(&cli)?;
            match a {
                AccountsCmd::List => {
                    let v = user(&engine, Command::ListAccounts).await?;
                    let accounts: Vec<Account> = serde_json::from_value(v)?;
                    for a in accounts {
                        println!("{}\t{}\t{:?}\t{:?}\t{}", a.id, a.email, a.kind, a.sync.state, a.sync.error.unwrap_or_default());
                    }
                }
                AccountsCmd::AddImap { email, name, username, password, imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, caldav_url, carddav_url, insecure } => {
                    let config = AccountConfig::Imap {
                        email: email.clone(),
                        display_name: name.clone(),
                        username: username.clone().unwrap_or_else(|| email.clone()),
                        imap_host: imap_host.clone(),
                        imap_port: *imap_port,
                        imap_security: security(imap_security)?,
                        smtp_host: smtp_host.clone().unwrap_or_else(|| imap_host.clone()),
                        smtp_port: *smtp_port,
                        smtp_security: security(smtp_security)?,
                        caldav_url: caldav_url.clone(),
                        carddav_url: carddav_url.clone(),
                        allow_invalid_certs: *insecure,
                    };
                    let v = user(&engine, Command::AddAccount { config, credential: Credential::Password { password: password.clone() } }).await?;
                    println!("{}", v["id"].as_str().unwrap_or_default());
                }
                AccountsCmd::AddJmap { email, name, username, password, session_url, caldav_url, carddav_url, insecure } => {
                    let config = AccountConfig::Jmap {
                        email: email.clone(),
                        display_name: name.clone(),
                        username: username.clone().unwrap_or_else(|| email.clone()),
                        session_url: session_url.clone(),
                        caldav_url: caldav_url.clone(),
                        carddav_url: carddav_url.clone(),
                        allow_invalid_certs: *insecure,
                    };
                    let v = user(&engine, Command::AddAccount { config, credential: Credential::Password { password: password.clone() } }).await?;
                    println!("{}", v["id"].as_str().unwrap_or_default());
                }
                AccountsCmd::Remove { account_id } => {
                    user(&engine, Command::RemoveAccount { account_id: account_id.clone() }).await?;
                }
            }
        }
        Cmd::Agents(a) => {
            let (engine, _lock) = open_for_call(&cli)?;
            match a {
                AgentsCmd::List => {
                    let v = user(&engine, Command::ListAgents).await?;
                    let agents: Vec<Agent> = serde_json::from_value(v)?;
                    for a in agents {
                        println!("{}\t{}\trevoked={}\t{:?}", a.id, a.name, a.revoked, a.policy.account_ids);
                    }
                }
                AgentsCmd::Create { name, accounts, since_days, allow_attachments } => {
                    let policy = AgentPolicy {
                        account_ids: accounts.as_ref().map(|s| s.split(',').map(|x| x.trim().to_string()).collect()),
                        since_days: since_days.or(AgentPolicy::default().since_days),
                        allow_attachments: *allow_attachments,
                        ..Default::default()
                    };
                    let v = user(&engine, Command::CreateAgent { name: name.clone(), policy }).await?;
                    println!("{}", v["token"].as_str().unwrap_or_default());
                }
                AgentsCmd::Revoke { agent_id } => {
                    user(&engine, Command::RevokeAgent { agent_id: agent_id.clone() }).await?;
                }
            }
        }
        Cmd::Call { json } => {
            let (engine, _lock) = open_for_call(&cli)?;
            let cmd: Command = serde_json::from_str(json).context("not a valid command (see satchel-api Command)")?;
            let v = user(&engine, cmd).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}

fn default_ui_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    for base in exe.ancestors().skip(1).take(5) {
        let p = base.join("ui/dist");
        if p.join("index.html").exists() {
            return Some(p);
        }
    }
    None
}
