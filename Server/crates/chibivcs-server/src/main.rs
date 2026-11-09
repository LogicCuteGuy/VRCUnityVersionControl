pub mod api;
pub mod blobs;
pub mod db;

use crate::api::{build_router, AppState};
use crate::blobs::BlobStore;
use crate::db::Database;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::info;

#[derive(Parser)]
#[command(name = "chibivcs-server", version, about = "ChibiVCS self-hosted Plastic-style VCS server")]
struct Cli {
    #[arg(long, default_value = "./data", global = true)]
    data_dir: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the HTTP/SSE server
    Serve {
        #[arg(long, default_value = "0.0.0.0:8087")]
        bind: String,
    },
    /// Manage user accounts
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Manage repositories
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },
}

#[derive(Subcommand)]
enum UserAction {
    /// Add or update a user
    Add {
        username: String,
        #[arg(long)]
        password: String,
        #[arg(long, default_value_t = false)]
        admin: bool,
    },
    /// Generate a bearer token for a user
    Token { username: String },
    /// List all users
    List,
}

#[derive(Subcommand)]
enum RepoAction {
    /// Create a repository
    Create {
        name: String,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// List all repositories
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "chibivcs_server=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data_dir)?;
    let db = Database::open(cli.data_dir.join("chibivcs.db"))?;
    let blobs = BlobStore::new(cli.data_dir.join("blobs"))?;

    match cli.command {
        Commands::Serve { bind } => {
            let (events_tx, _) = broadcast::channel(256);
            let state = AppState {
                db,
                blobs,
                events_tx,
            };
            let app = build_router(state).layer(CorsLayer::permissive());
            let addr: SocketAddr = bind.parse().context("invalid bind address")?;
            info!("ChibiVCS server listening on http://{}", addr);
            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app).await?;
        }
        Commands::User { action } => match action {
            UserAction::Add {
                username,
                password,
                admin,
            } => {
                db.upsert_user(&username, &password, admin)?;
                println!(
                    "User '{}' saved (admin={})",
                    username, admin
                );
            }
            UserAction::Token { username } => {
                let token = db.create_token_for_user(&username)?;
                println!("{}", token);
            }
            UserAction::List => {
                for (u, admin, ts) in db.list_users()? {
                    println!("{}\tadmin={}\tcreated={}", u, admin, ts);
                }
            }
        },
        Commands::Repo { action } => match action {
            RepoAction::Create { name, description } => {
                let r = db.create_repo(&name, &description, "admin")?;
                println!(
                    "Repository '{}' ready (head=cs:{})",
                    r.name, r.head_changeset_id
                );
            }
            RepoAction::List => {
                for r in db.list_repos()? {
                    println!("{}\thead=cs:{}\t{}", r.name, r.head_changeset_id, r.description);
                }
            }
        },
    }

    Ok(())
}
