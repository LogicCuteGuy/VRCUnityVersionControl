use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub name: String,
    pub description: String,
    pub head_changeset_id: i64,
    pub created_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRepoRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub head_changeset_id: i64,
    pub created_by: String,
    pub created_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateBranchRequest {
    pub name: String,
    pub from_changeset_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeItem {
    pub path: String,
    pub kind: ChangeKind,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinRequest {
    pub branch: String,
    pub parent_changeset_id: i64,
    pub comment: String,
    pub changes: Vec<FileChangeItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinResponse {
    pub changeset_id: i64,
    pub branch: String,
    pub rebased: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetSummary {
    pub id: i64,
    pub branch: String,
    pub parent_id: i64,
    pub author: String,
    pub comment: String,
    pub timestamp_unix: i64,
    pub change_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangesetDetail {
    pub summary: ChangesetSummary,
    pub changes: Vec<FileChangeItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeResponse {
    pub changeset_id: i64,
    pub branch: String,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingBlobsRequest {
    pub hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingBlobsResponse {
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockInfo {
    pub path: String,
    pub branch: String,
    pub owner: String,
    pub locked_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquireLocksRequest {
    #[serde(default = "default_branch")]
    pub branch: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseLocksRequest {
    #[serde(default = "default_branch")]
    pub branch: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub force: bool,
}

fn default_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcsEvent {
    pub kind: String, // "changeset" | "lock" | "unlock"
    pub repo: String,
    pub branch: String,
    pub changeset_id: i64,
    pub paths: Vec<String>,
    pub user: String,
    pub message: String,
    pub timestamp_unix: i64,
}

/// Normalizes a relative workspace path to forward slashes without leading/trailing slashes.
/// Rejects path traversal (`..`).
pub fn normalize_rel_path(raw: &str) -> Option<String> {
    let unified = raw.replace('\\', "/");
    let trimmed = unified.trim().trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        parts.push(segment);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Computes lowercase hex SHA-256 of a byte slice.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Default Unity + VRChat ignore check for relative workspace paths.
pub fn is_ignored_unity_path(norm_path: &str) -> bool {
    let lower = norm_path.to_ascii_lowercase();
    let first_seg = lower.split('/').next().unwrap_or("");
    matches!(
        first_seg,
        "library"
            | "temp"
            | "obj"
            | "build"
            | "builds"
            | "logs"
            | "usersettings"
            | "memorycaptures"
            | "recordings"
            | ".git"
            | ".vs"
            | ".vscode"
            | ".idea"
            | ".chibivcs"
            | "server"
    ) || lower.starts_with("packages/com.vrchat.")
        || lower.starts_with("assets/serializedudonprograms/")
        || lower.ends_with(".csproj")
        || lower.ends_with(".sln")
        || lower.ends_with(".suo")
        || lower.ends_with(".user")
        || lower.ends_with(".userprefs")
        || lower.ends_with(".pidb")
        || lower.ends_with(".booproj")
        || lower.ends_with(".svd")
        || lower.ends_with(".pdb")
        || lower.ends_with(".mdb")
        || lower.ends_with(".opendb")
        || lower.ends_with(".vc.db")
        || lower.ends_with(".ds_store")
        || lower == "thumbs.db"
}

/// Returns true if a file is a binary/serialized Unity asset that should be exclusively locked on checkout.
pub fn is_lockable_unity_asset(norm_path: &str) -> bool {
    let lower = norm_path.to_ascii_lowercase();
    const LOCKABLE_EXTS: &[&str] = &[
        ".unity",
        ".prefab",
        ".asset",
        ".controller",
        ".anim",
        ".overridecontroller",
        ".mask",
        ".lighting",
        ".terrainlayer",
        ".mixer",
        ".fbx",
        ".blend",
        ".obj",
        ".dae",
        ".png",
        ".psd",
        ".tga",
        ".jpg",
        ".jpeg",
        ".exr",
        ".hdr",
        ".tif",
        ".tiff",
        ".wav",
        ".mp3",
        ".ogg",
        ".mp4",
        ".webm",
    ];
    LOCKABLE_EXTS.iter().any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_rel_path() {
        assert_eq!(
            normalize_rel_path(r"Assets\Scenes\Main.unity"),
            Some("Assets/Scenes/Main.unity".to_string())
        );
        assert_eq!(normalize_rel_path("../secret.txt"), None);
        assert_eq!(normalize_rel_path("Assets/../../secret"), None);
        assert_eq!(normalize_rel_path(""), None);
    }

    #[test]
    fn test_ignore_and_lockable() {
        assert!(is_ignored_unity_path("Library/ArtifactDB"));
        assert!(is_ignored_unity_path("Temp/UnityLockfile"));
        assert!(is_ignored_unity_path("Assembly-CSharp.csproj"));
        assert!(!is_ignored_unity_path("Assets/Scenes/VRCDefaultWorldScene.unity"));
        assert!(is_lockable_unity_asset("Assets/Scenes/VRCDefaultWorldScene.unity"));
        assert!(is_lockable_unity_asset("Assets/Prefabs/Stage.prefab"));
        assert!(!is_lockable_unity_asset("Assets/Scripts/MyUdon.cs"));
    }
}
