use crate::blobs::BlobStore;
use crate::db::{now_unix, AuthUser, Database, VcsError};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post, put},
    Json, Router,
};
use chibivcs_proto::*;
use futures_util::stream::Stream;
use serde::Deserialize;
use serde_json::json;
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub blobs: BlobStore,
    pub events_tx: broadcast::Sender<VcsEvent>,
}

impl IntoResponse for VcsError {
    fn into_response(self) -> Response {
        let status = match &self {
            VcsError::Unauthorized => StatusCode::UNAUTHORIZED,
            VcsError::Forbidden => StatusCode::FORBIDDEN,
            VcsError::NotFound(_) => StatusCode::NOT_FOUND,
            VcsError::LockedByOther { .. } => StatusCode::CONFLICT,
            VcsError::NeedsUpdate { .. } => StatusCode::CONFLICT,
            VcsError::MissingBlob(_) => StatusCode::PRECONDITION_FAILED,
            VcsError::BadRequest(_) => StatusCode::BAD_REQUEST,
            VcsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({
            "error": self.to_string(),
        }));
        (status, body).into_response()
    }
}

fn require_user(state: &AppState, headers: &HeaderMap) -> Result<AuthUser, VcsError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(VcsError::Unauthorized)?;
    let token = auth_header
        .strip_prefix("Bearer ")
        .or_else(|| auth_header.strip_prefix("bearer "))
        .unwrap_or(auth_header)
        .trim();
    if token.is_empty() {
        return Err(VcsError::Unauthorized);
    }
    state
        .db
        .authenticate_token(token)
        .map_err(VcsError::Internal)?
        .ok_or(VcsError::Unauthorized)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health_handler))
        .route("/api/v1/login", post(login_handler))
        .route("/api/v1/repos", get(list_repos_handler).post(create_repo_handler))
        .route(
            "/api/v1/repos/:repo/branches",
            get(list_branches_handler).post(create_branch_handler),
        )
        .route("/api/v1/repos/:repo/changesets", get(list_changesets_handler))
        .route("/api/v1/repos/:repo/changesets/:id", get(get_changeset_handler))
        .route(
            "/api/v1/repos/:repo/changesets/:id/tree",
            get(get_tree_handler),
        )
        .route(
            "/api/v1/repos/:repo/blobs/missing",
            post(missing_blobs_handler),
        )
        .route(
            "/api/v1/repos/:repo/blobs/:sha256",
            put(put_blob_handler).get(get_blob_handler),
        )
        .route("/api/v1/repos/:repo/checkin", post(checkin_handler))
        .route(
            "/api/v1/repos/:repo/locks",
            get(list_locks_handler)
                .post(acquire_locks_handler)
                .delete(release_locks_handler),
        )
        .route(
            "/api/v1/repos/:repo/locks/unlock",
            post(release_locks_handler),
        )
        .route("/api/v1/repos/:repo/events", get(events_sse_handler))
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .with_state(state)
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "service": "chibivcs-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn login_handler(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, VcsError> {
    let Some(resp) = state.db.verify_login(&req.username, &req.password)? else {
        return Err(VcsError::Unauthorized);
    };
    Ok(Json(resp))
}

async fn list_repos_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RepoInfo>>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let repos = state.db.list_repos()?;
    Ok(Json(repos))
}

async fn create_repo_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRepoRequest>,
) -> Result<Json<RepoInfo>, VcsError> {
    let user = require_user(&state, &headers)?;
    if !user.is_admin {
        return Err(VcsError::Forbidden);
    }
    let repo = state
        .db
        .create_repo(&req.name, &req.description, &user.username)?;
    Ok(Json(repo))
}

async fn list_branches_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
) -> Result<Json<Vec<BranchInfo>>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let branches = state.db.list_branches(&repo)?;
    Ok(Json(branches))
}

async fn create_branch_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Json(req): Json<CreateBranchRequest>,
) -> Result<Json<BranchInfo>, VcsError> {
    let user = require_user(&state, &headers)?;
    let branch = state
        .db
        .create_branch(&repo, &req.name, req.from_changeset_id, &user.username)?;
    Ok(Json(branch))
}

#[derive(Deserialize, Default)]
struct ChangesetsQuery {
    branch: Option<String>,
    after: Option<i64>,
    limit: Option<usize>,
}

async fn list_changesets_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Query(q): Query<ChangesetsQuery>,
) -> Result<Json<Vec<ChangesetSummary>>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let list = state.db.list_changesets(
        &repo,
        q.branch.as_deref(),
        q.after,
        q.limit.unwrap_or(100),
    )?;
    Ok(Json(list))
}

async fn get_changeset_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((repo, id)): Path<(String, i64)>,
) -> Result<Json<ChangesetDetail>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let detail = state.db.get_changeset(&repo, id)?;
    Ok(Json(detail))
}

#[derive(Deserialize, Default)]
struct TreeQuery {
    branch: Option<String>,
}

async fn get_tree_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((repo, id_str)): Path<(String, String)>,
    Query(q): Query<TreeQuery>,
) -> Result<Json<TreeResponse>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let cs_id = if id_str.eq_ignore_ascii_case("latest") {
        -1
    } else {
        id_str
            .parse::<i64>()
            .map_err(|_| VcsError::BadRequest(format!("invalid changeset id: {}", id_str)))?
    };
    let tree = state.db.get_tree(&repo, cs_id, q.branch.as_deref())?;
    Ok(Json(tree))
}

async fn missing_blobs_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_repo): Path<String>,
    Json(req): Json<MissingBlobsRequest>,
) -> Result<Json<MissingBlobsResponse>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let mut missing = Vec::new();
    for h in req.hashes {
        let clean = BlobStore::validate_hash(&h).map_err(|e| VcsError::BadRequest(e.to_string()))?;
        if !state.blobs.has(&clean) {
            missing.push(clean);
        }
    }
    Ok(Json(MissingBlobsResponse { missing }))
}

async fn put_blob_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_repo, sha256)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, VcsError> {
    let _user = require_user(&state, &headers)?;
    state
        .blobs
        .put(&sha256, &body)
        .map_err(|e| VcsError::BadRequest(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "sha256": sha256, "size": body.len() })))
}

async fn get_blob_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_repo, sha256)): Path<(String, String)>,
) -> Result<Response, VcsError> {
    let _user = require_user(&state, &headers)?;
    let Some(bytes) = state
        .blobs
        .get(&sha256)
        .map_err(|e| VcsError::BadRequest(e.to_string()))?
    else {
        return Err(VcsError::NotFound(format!("blob {}", sha256)));
    };
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response())
}

async fn checkin_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Json(req): Json<CheckinRequest>,
) -> Result<Json<CheckinResponse>, VcsError> {
    let user = require_user(&state, &headers)?;
    let comment = req.comment.clone();
    let changed_paths: Vec<String> = req.changes.iter().map(|c| c.path.clone()).collect();
    let resp = state.db.checkin(&repo, &user.username, req, &state.blobs)?;

    let _ = state.events_tx.send(VcsEvent {
        kind: "changeset".into(),
        repo,
        branch: resp.branch.clone(),
        changeset_id: resp.changeset_id,
        paths: changed_paths,
        user: user.username,
        message: comment,
        timestamp_unix: now_unix(),
    });

    Ok(Json(resp))
}

#[derive(Deserialize, Default)]
struct LocksQuery {
    branch: Option<String>,
}

async fn list_locks_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Query(q): Query<LocksQuery>,
) -> Result<Json<Vec<LockInfo>>, VcsError> {
    let _user = require_user(&state, &headers)?;
    let locks = state.db.list_locks(&repo, q.branch.as_deref())?;
    Ok(Json(locks))
}

async fn acquire_locks_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Json(req): Json<AcquireLocksRequest>,
) -> Result<Json<Vec<LockInfo>>, VcsError> {
    let user = require_user(&state, &headers)?;
    let locks = state
        .db
        .acquire_locks(&repo, &req.branch, &user.username, &req.paths)?;
    let paths: Vec<String> = locks.iter().map(|l| l.path.clone()).collect();
    let _ = state.events_tx.send(VcsEvent {
        kind: "lock".into(),
        repo,
        branch: req.branch,
        changeset_id: 0,
        paths,
        user: user.username,
        message: "locked".into(),
        timestamp_unix: now_unix(),
    });
    Ok(Json(locks))
}

async fn release_locks_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Json(req): Json<ReleaseLocksRequest>,
) -> Result<Json<Vec<String>>, VcsError> {
    let user = require_user(&state, &headers)?;
    let released = state
        .db
        .release_locks(&repo, &req.branch, &user, &req.paths, req.force)?;
    if !released.is_empty() {
        let _ = state.events_tx.send(VcsEvent {
            kind: "unlock".into(),
            repo,
            branch: req.branch,
            changeset_id: 0,
            paths: released.clone(),
            user: user.username,
            message: "unlocked".into(),
            timestamp_unix: now_unix(),
        });
    }
    Ok(Json(released))
}

#[derive(Deserialize, Default)]
struct EventsQuery {
    token: Option<String>,
}

async fn events_sse_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(repo): Path<String>,
    Query(q): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, VcsError> {
    if let Some(tok) = q.token.as_deref().filter(|s| !s.is_empty()) {
        state
            .db
            .authenticate_token(tok)
            .map_err(VcsError::Internal)?
            .ok_or(VcsError::Unauthorized)?;
    } else {
        require_user(&state, &headers)?;
    }

    let rx = state.events_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |msg| {
        if let Ok(ev) = msg {
            if ev.repo.eq_ignore_ascii_case(&repo) {
                if let Ok(data) = serde_json::to_string(&ev) {
                    return Some(Ok(Event::default().event(ev.kind).data(data)));
                }
            }
        }
        None
    });

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}
