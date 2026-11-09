use anyhow::{bail, Context, Result};
use chibivcs_proto::*;
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceConfig {
    pub server_url: String,
    pub repo: String,
    pub branch: String,
    pub base_changeset_id: i64,
    pub username: String,
    pub token: String,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

#[derive(Parser)]
#[command(name = "chibi", version, about = "ChibiVCS command-line client")]
struct Cli {
    #[arg(long, default_value = ".", global = true)]
    dir: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Login and initialize or attach workspace to a repository
    Init {
        #[arg(long)]
        server: String,
        #[arg(long)]
        repo: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
    },
    /// Show pending local changes
    Status,
    /// Upload modified blobs and create a new changeset
    Checkin {
        #[arg(short, long)]
        message: String,
    },
    /// Update workspace to latest changeset on current branch
    Update,
    /// Acquire exclusive locks on paths
    Lock { paths: Vec<String> },
    /// Release locks on paths
    Unlock {
        #[arg(long, default_value_t = false)]
        force: bool,
        paths: Vec<String>,
    },
    /// List active locks on repository
    Locks,
}

fn ws_file(root: &Path) -> PathBuf {
    root.join(".chibivcs").join("workspace.json")
}

fn load_ws(root: &Path) -> Result<WorkspaceConfig> {
    let path = ws_file(root);
    let data = fs::read_to_string(&path)
        .with_context(|| format!("workspace not initialized at {}", path.display()))?;
    Ok(serde_json::from_str(&data)?)
}

fn save_ws(root: &Path, ws: &WorkspaceConfig) -> Result<()> {
    let dir = root.join(".chibivcs");
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(ws)?;
    fs::write(ws_file(root), json)?;
    Ok(())
}

fn scan_disk(root: &Path, cur: &Path, out: &mut HashMap<String, (String, u64)>) -> Result<()> {
    for entry in fs::read_dir(cur)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let Some(norm) = normalize_rel_path(&rel.to_string_lossy()) else {
            continue;
        };
        if is_ignored_unity_path(&norm) {
            continue;
        }
        let meta = entry.metadata()?;
        if meta.is_dir() {
            scan_disk(root, &path, out)?;
        } else if meta.is_file() {
            let bytes = fs::read(&path)?;
            let hash = sha256_hex(&bytes);
            out.insert(norm, (hash, bytes.len() as u64));
        }
    }
    Ok(())
}

fn compute_pending(root: &Path, ws: &WorkspaceConfig) -> Result<(Vec<FileChangeItem>, HashMap<String, (String, u64)>)> {
    let mut disk = HashMap::new();
    scan_disk(root, root, &mut disk)?;

    let mut base_map = HashMap::new();
    for f in &ws.files {
        base_map.insert(f.path.clone(), (f.sha256.clone(), f.size));
    }

    let mut changes = Vec::new();
    for (path, (sha, size)) in &disk {
        match base_map.get(path) {
            None => changes.push(FileChangeItem {
                path: path.clone(),
                kind: ChangeKind::Added,
                sha256: sha.clone(),
                size: *size,
            }),
            Some((base_sha, _)) if base_sha != sha => changes.push(FileChangeItem {
                path: path.clone(),
                kind: ChangeKind::Modified,
                sha256: sha.clone(),
                size: *size,
            }),
            _ => {}
        }
    }

    for path in base_map.keys() {
        if !disk.contains_key(path) {
            changes.push(FileChangeItem {
                path: path.clone(),
                kind: ChangeKind::Deleted,
                sha256: String::new(),
                size: 0,
            });
        }
    }

    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((changes, disk))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = fs::canonicalize(&cli.dir).unwrap_or(cli.dir.clone());
    let client = Client::new();

    match cli.command {
        Commands::Init {
            server,
            repo,
            branch,
            username,
            password,
        } => {
            let server = server.trim_end_matches('/').to_string();
            let resp = client
                .post(format!("{}/api/v1/login", server))
                .json(&LoginRequest {
                    username: username.clone(),
                    password,
                })
                .send()?;
            if !resp.status().is_success() {
                bail!("login failed: HTTP {}", resp.status());
            }
            let login: LoginResponse = resp.json()?;
            let ws = WorkspaceConfig {
                server_url: server,
                repo,
                branch,
                base_changeset_id: 0,
                username: login.username,
                token: login.token,
                files: Vec::new(),
            };
            save_ws(&root, &ws)?;
            println!("Initialized ChibiVCS workspace in {}", root.display());
        }
        Commands::Status => {
            let ws = load_ws(&root)?;
            let (changes, _) = compute_pending(&root, &ws)?;
            println!(
                "Workspace: {}@{} (cs:{}) user={}",
                ws.repo, ws.branch, ws.base_changeset_id, ws.username
            );
            if changes.is_empty() {
                println!("Clean workspace (0 pending changes).");
            } else {
                for ch in &changes {
                    let tag = match ch.kind {
                        ChangeKind::Added => "A",
                        ChangeKind::Modified => "M",
                        ChangeKind::Deleted => "D",
                    };
                    println!("  [{}] {}", tag, ch.path);
                }
                println!("Total: {} pending change(s)", changes.len());
            }
        }
        Commands::Checkin { message } => {
            let mut ws = load_ws(&root)?;
            let (changes, disk) = compute_pending(&root, &ws)?;
            if changes.is_empty() {
                println!("Nothing to checkin.");
                return Ok(());
            }

            let hashes: Vec<String> = changes
                .iter()
                .filter(|c| c.kind != ChangeKind::Deleted)
                .map(|c| c.sha256.clone())
                .collect();

            let miss_resp = client
                .post(format!(
                    "{}/api/v1/repos/{}/blobs/missing",
                    ws.server_url, ws.repo
                ))
                .bearer_auth(&ws.token)
                .json(&MissingBlobsRequest { hashes })
                .send()?;
            if !miss_resp.status().is_success() {
                bail!("missing check failed: {}", miss_resp.text()?);
            }
            let missing: MissingBlobsResponse = miss_resp.json()?;
            let missing_set: std::collections::HashSet<String> =
                missing.missing.into_iter().collect();

            for ch in &changes {
                if ch.kind != ChangeKind::Deleted && missing_set.contains(&ch.sha256) {
                    let bytes = fs::read(root.join(&ch.path))?;
                    let put_resp = client
                        .put(format!(
                            "{}/api/v1/repos/{}/blobs/{}",
                            ws.server_url, ws.repo, ch.sha256
                        ))
                        .bearer_auth(&ws.token)
                        .body(bytes)
                        .send()?;
                    if !put_resp.status().is_success() {
                        bail!("blob upload failed for {}: {}", ch.path, put_resp.text()?);
                    }
                }
            }

            let ci_resp = client
                .post(format!(
                    "{}/api/v1/repos/{}/checkin",
                    ws.server_url, ws.repo
                ))
                .bearer_auth(&ws.token)
                .json(&CheckinRequest {
                    branch: ws.branch.clone(),
                    parent_changeset_id: ws.base_changeset_id,
                    comment: message,
                    changes,
                })
                .send()?;
            if !ci_resp.status().is_success() {
                bail!("checkin failed: {}", ci_resp.text()?);
            }
            let ci: CheckinResponse = ci_resp.json()?;
            ws.base_changeset_id = ci.changeset_id;
            ws.files = disk
                .into_iter()
                .map(|(path, (sha256, size))| FileEntry { path, sha256, size })
                .collect();
            ws.files.sort_by(|a, b| a.path.cmp(&b.path));
            save_ws(&root, &ws)?;
            println!(
                "Checked in cs:{} on {} (rebased={})",
                ci.changeset_id, ci.branch, ci.rebased
            );
        }
        Commands::Update => {
            let mut ws = load_ws(&root)?;
            let tree_resp = client
                .get(format!(
                    "{}/api/v1/repos/{}/changesets/latest/tree?branch={}",
                    ws.server_url, ws.repo, ws.branch
                ))
                .bearer_auth(&ws.token)
                .send()?;
            if !tree_resp.status().is_success() {
                bail!("tree fetch failed: {}", tree_resp.text()?);
            }
            let tree: TreeResponse = tree_resp.json()?;

            let remote_paths: std::collections::HashSet<String> =
                tree.files.iter().map(|f| f.path.clone()).collect();
            for old in &ws.files {
                if !remote_paths.contains(&old.path) {
                    let p = root.join(&old.path);
                    if p.is_file() {
                        let _ = fs::remove_file(p);
                    }
                }
            }

            for f in &tree.files {
                let target = root.join(&f.path);
                let need_download = if target.is_file() {
                    let existing = fs::read(&target)?;
                    sha256_hex(&existing) != f.sha256
                } else {
                    true
                };
                if need_download {
                    let blob_resp = client
                        .get(format!(
                            "{}/api/v1/repos/{}/blobs/{}",
                            ws.server_url, ws.repo, f.sha256
                        ))
                        .bearer_auth(&ws.token)
                        .send()?;
                    if !blob_resp.status().is_success() {
                        bail!("download blob failed for {}: {}", f.path, blob_resp.status());
                    }
                    let bytes = blob_resp.bytes()?;
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&target, &bytes)?;
                }
            }

            ws.base_changeset_id = tree.changeset_id;
            ws.files = tree.files;
            save_ws(&root, &ws)?;
            println!("Updated workspace to cs:{}", ws.base_changeset_id);
        }
        Commands::Lock { paths } => {
            let ws = load_ws(&root)?;
            let resp = client
                .post(format!("{}/api/v1/repos/{}/locks", ws.server_url, ws.repo))
                .bearer_auth(&ws.token)
                .json(&AcquireLocksRequest {
                    branch: ws.branch.clone(),
                    paths,
                })
                .send()?;
            if !resp.status().is_success() {
                bail!("lock failed: {}", resp.text()?);
            }
            let locked: Vec<LockInfo> = resp.json()?;
            for l in locked {
                println!("Locked: {} (by {})", l.path, l.owner);
            }
        }
        Commands::Unlock { force, paths } => {
            let ws = load_ws(&root)?;
            let resp = client
                .post(format!(
                    "{}/api/v1/repos/{}/locks/unlock",
                    ws.server_url, ws.repo
                ))
                .bearer_auth(&ws.token)
                .json(&ReleaseLocksRequest {
                    branch: ws.branch.clone(),
                    paths,
                    force,
                })
                .send()?;
            if !resp.status().is_success() {
                bail!("unlock failed: {}", resp.text()?);
            }
            let released: Vec<String> = resp.json()?;
            for p in released {
                println!("Unlocked: {}", p);
            }
        }
        Commands::Locks => {
            let ws = load_ws(&root)?;
            let resp = client
                .get(format!(
                    "{}/api/v1/repos/{}/locks?branch={}",
                    ws.server_url, ws.repo, ws.branch
                ))
                .bearer_auth(&ws.token)
                .send()?;
            if !resp.status().is_success() {
                bail!("list locks failed: {}", resp.text()?);
            }
            let locks: Vec<LockInfo> = resp.json()?;
            if locks.is_empty() {
                println!("No active locks.");
            } else {
                for l in locks {
                    println!("{}\t{}\t{}", l.owner, l.branch, l.path);
                }
            }
        }
    }

    Ok(())
}
