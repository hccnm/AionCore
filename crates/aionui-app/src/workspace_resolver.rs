use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use aionui_db::{IWorkspaceRepository, WorkspaceRow};

#[derive(Clone)]
pub struct WorkspaceResolver {
    senmo_root: PathBuf,
    workspace_repo: Arc<dyn IWorkspaceRepository>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceResolveMode {
    Existing,
    ParentForCreate,
}

#[derive(Debug, Clone)]
pub struct ResolvedWorkspacePath {
    pub workspace: WorkspaceRow,
    pub workspace_root: PathBuf,
    pub absolute_path: PathBuf,
    pub relative_path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceResolveError {
    #[error("workspace lookup failed: {0}")]
    Lookup(String),
    #[error("workspace not found: {0}")]
    NotFound(String),
    #[error("workspace access denied: {0}")]
    Forbidden(String),
    #[error("invalid workspace path: {0}")]
    BadPath(String),
    #[error("workspace resolver internal error: {0}")]
    Internal(String),
}

impl WorkspaceResolver {
    pub fn new(senmo_root: PathBuf, workspace_repo: Arc<dyn IWorkspaceRepository>) -> Self {
        Self {
            senmo_root,
            workspace_repo,
        }
    }

    pub async fn resolve_for_user(
        &self,
        owner_user_id: &str,
        workspace_id: &str,
        relative_path: &str,
        mode: WorkspaceResolveMode,
    ) -> Result<ResolvedWorkspacePath, WorkspaceResolveError> {
        let workspace = self
            .workspace_repo
            .find_by_id(workspace_id)
            .await
            .map_err(|error| WorkspaceResolveError::Lookup(error.to_string()))?
            .ok_or_else(|| WorkspaceResolveError::NotFound(format!("Workspace '{workspace_id}' not found")))?;

        if workspace.owner_user_id != owner_user_id {
            return Err(WorkspaceResolveError::Forbidden(
                "Workspace is not owned by current user".into(),
            ));
        }
        if !matches!(workspace.status.as_str(), "active" | "provisioning") {
            return Err(WorkspaceResolveError::Forbidden("Workspace is not active".into()));
        }

        self.resolve_workspace_path(workspace, relative_path, mode)
    }

    pub fn resolve_workspace_path(
        &self,
        workspace: WorkspaceRow,
        relative_path: &str,
        mode: WorkspaceResolveMode,
    ) -> Result<ResolvedWorkspacePath, WorkspaceResolveError> {
        let relative = normalize_relative_path(relative_path)?;
        let senmo_root = self.senmo_root.canonicalize().map_err(|error| {
            WorkspaceResolveError::Internal(format!("SENMO_WORKSPACE_ROOT is not accessible: {error}"))
        })?;
        let workspace_root = safe_join_under_root(&senmo_root, &workspace.root_rel_path)?;
        let canonical_workspace_root = workspace_root
            .canonicalize()
            .map_err(|error| WorkspaceResolveError::NotFound(format!("Workspace root is not accessible: {error}")))?;
        if !canonical_workspace_root.starts_with(&senmo_root) {
            return Err(WorkspaceResolveError::Forbidden(
                "Workspace root escaped SENMO_WORKSPACE_ROOT".into(),
            ));
        }

        let candidate = canonical_workspace_root.join(&relative);
        let canonical_check = match mode {
            WorkspaceResolveMode::Existing => candidate
                .canonicalize()
                .map_err(|error| WorkspaceResolveError::BadPath(format!("Path is not accessible: {error}")))?,
            WorkspaceResolveMode::ParentForCreate => {
                let parent = candidate
                    .parent()
                    .ok_or_else(|| WorkspaceResolveError::BadPath("Path has no parent".into()))?;
                nearest_existing_ancestor(parent, &canonical_workspace_root)?
                    .canonicalize()
                    .map_err(|error| WorkspaceResolveError::BadPath(format!("Path is not accessible: {error}")))?
            }
        };
        if !canonical_check.starts_with(&canonical_workspace_root) {
            return Err(WorkspaceResolveError::Forbidden("Path escaped workspace root".into()));
        }

        Ok(ResolvedWorkspacePath {
            workspace,
            workspace_root: canonical_workspace_root,
            absolute_path: match mode {
                WorkspaceResolveMode::Existing => canonical_check,
                WorkspaceResolveMode::ParentForCreate => candidate,
            },
            relative_path: relative.to_string_lossy().replace('\\', "/"),
        })
    }
}

pub fn safe_join_under_root(root: &Path, relative_path: &str) -> Result<PathBuf, WorkspaceResolveError> {
    let relative = normalize_relative_path(relative_path)?;
    let (canonical_root, root_exists) = canonicalize_root_or_creatable(root)?;
    let candidate = canonical_root.join(&relative);

    if !root_exists {
        return Ok(candidate);
    }

    if candidate.exists() {
        let canonical_candidate = candidate
            .canonicalize()
            .map_err(|error| WorkspaceResolveError::BadPath(format!("Path is not accessible: {error}")))?;
        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(WorkspaceResolveError::Forbidden("Path escaped root".into()));
        }
        return Ok(canonical_candidate);
    }

    let parent = candidate
        .parent()
        .ok_or_else(|| WorkspaceResolveError::BadPath("Path has no parent".into()))?;
    let canonical_parent = nearest_existing_ancestor(parent, &canonical_root)?
        .canonicalize()
        .map_err(|error| WorkspaceResolveError::BadPath(format!("Path parent is not accessible: {error}")))?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(WorkspaceResolveError::Forbidden("Path escaped root".into()));
    }
    Ok(candidate)
}

fn canonicalize_root_or_creatable(root: &Path) -> Result<(PathBuf, bool), WorkspaceResolveError> {
    if root.exists() {
        return root
            .canonicalize()
            .map(|path| (path, true))
            .map_err(|error| WorkspaceResolveError::Internal(format!("Root path is not accessible: {error}")));
    }

    let parent = root
        .parent()
        .ok_or_else(|| WorkspaceResolveError::Internal("Root path has no parent".into()))?;
    let parent = parent
        .canonicalize()
        .map_err(|error| WorkspaceResolveError::Internal(format!("Root parent path is not accessible: {error}")))?;
    let name = root
        .file_name()
        .ok_or_else(|| WorkspaceResolveError::Internal("Root path has no final component".into()))?;
    Ok((parent.join(name), false))
}

fn nearest_existing_ancestor<'a>(path: &'a Path, workspace_root: &'a Path) -> Result<&'a Path, WorkspaceResolveError> {
    let mut current = path;
    loop {
        if current.exists() {
            return Ok(current);
        }
        if current == workspace_root {
            return Ok(workspace_root);
        }
        current = current
            .parent()
            .ok_or_else(|| WorkspaceResolveError::BadPath("Path has no existing parent".into()))?;
    }
}

fn normalize_relative_path(value: &str) -> Result<PathBuf, WorkspaceResolveError> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(WorkspaceResolveError::BadPath(
            "Path must be relative to workspace".into(),
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WorkspaceResolveError::BadPath("Path traversal is not allowed".into()));
            }
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aionui_db::{CreateWorkspaceParams, DbError};
    use tempfile::tempdir;
    use tokio::sync::RwLock;

    #[derive(Default)]
    struct FakeWorkspaceRepo {
        workspace: RwLock<Option<WorkspaceRow>>,
    }

    #[async_trait::async_trait]
    impl IWorkspaceRepository for FakeWorkspaceRepo {
        async fn create_workspace(&self, _params: CreateWorkspaceParams) -> Result<WorkspaceRow, DbError> {
            unreachable!("not needed")
        }

        async fn find_by_id(&self, workspace_id: &str) -> Result<Option<WorkspaceRow>, DbError> {
            Ok(self
                .workspace
                .read()
                .await
                .clone()
                .filter(|workspace| workspace.id == workspace_id))
        }

        async fn list_by_owner(&self, _owner_user_id: &str) -> Result<Vec<WorkspaceRow>, DbError> {
            unreachable!("not needed")
        }

        async fn has_active_by_git_project(&self, _git_project_id: &str) -> Result<bool, DbError> {
            unreachable!("not needed")
        }

        async fn update_name(&self, _workspace_id: &str, _name: &str) -> Result<(), DbError> {
            unreachable!("not needed")
        }

        async fn update_status(&self, _workspace_id: &str, _status: &str) -> Result<(), DbError> {
            unreachable!("not needed")
        }

        async fn mark_deleted(&self, _workspace_id: &str) -> Result<(), DbError> {
            unreachable!("not needed")
        }
    }

    fn row(owner_user_id: &str, root_rel_path: &str) -> WorkspaceRow {
        WorkspaceRow {
            id: "ws_1".into(),
            owner_user_id: owner_user_id.into(),
            name: "demo".into(),
            source_type: "blank".into(),
            git_project_id: None,
            branch_ref: None,
            status: "active".into(),
            root_rel_path: root_rel_path.into(),
            created_at: 1,
            updated_at: 1,
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn resolves_relative_path_inside_owned_workspace() {
        let senmo = tempdir().unwrap();
        let workspace_root = senmo.path().join("u1/ws_1");
        std::fs::create_dir_all(workspace_root.join("src")).unwrap();
        std::fs::write(workspace_root.join("src/main.rs"), "fn main() {}").unwrap();
        let repo = Arc::new(FakeWorkspaceRepo::default());
        *repo.workspace.write().await = Some(row("u1", "u1/ws_1"));
        let resolver = WorkspaceResolver::new(senmo.path().to_path_buf(), repo);

        let resolved = resolver
            .resolve_for_user("u1", "ws_1", "src/main.rs", WorkspaceResolveMode::Existing)
            .await
            .unwrap();

        assert_eq!(resolved.relative_path, "src/main.rs");
        assert!(resolved.absolute_path.ends_with("src/main.rs"));
    }

    #[tokio::test]
    async fn rejects_traversal_and_absolute_path() {
        let senmo = tempdir().unwrap();
        std::fs::create_dir_all(senmo.path().join("u1/ws_1")).unwrap();
        let repo = Arc::new(FakeWorkspaceRepo::default());
        *repo.workspace.write().await = Some(row("u1", "u1/ws_1"));
        let resolver = WorkspaceResolver::new(senmo.path().to_path_buf(), repo);

        assert!(
            resolver
                .resolve_for_user("u1", "ws_1", "../secret", WorkspaceResolveMode::Existing)
                .await
                .is_err()
        );
        assert!(
            resolver
                .resolve_for_user("u1", "ws_1", "/etc/passwd", WorkspaceResolveMode::Existing)
                .await
                .is_err()
        );
    }

    #[test]
    fn safe_join_allows_creating_under_missing_senmo_root() {
        let parent = tempdir().unwrap();
        let senmo = parent.path().join("senmo-missing");

        let resolved = safe_join_under_root(&senmo, "u1/ws_1").unwrap();

        assert_eq!(
            resolved,
            parent.path().canonicalize().unwrap().join("senmo-missing/u1/ws_1")
        );
    }

    #[tokio::test]
    async fn rejects_cross_owner_access() {
        let senmo = tempdir().unwrap();
        std::fs::create_dir_all(senmo.path().join("u1/ws_1")).unwrap();
        let repo = Arc::new(FakeWorkspaceRepo::default());
        *repo.workspace.write().await = Some(row("u1", "u1/ws_1"));
        let resolver = WorkspaceResolver::new(senmo.path().to_path_buf(), repo);

        let error = resolver
            .resolve_for_user("u2", "ws_1", "", WorkspaceResolveMode::Existing)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not owned"));
    }

    #[tokio::test]
    async fn resolved_response_path_does_not_leak_senmo_root() {
        let senmo = tempdir().unwrap();
        let workspace_root = senmo.path().join("u1/ws_1");
        std::fs::create_dir_all(workspace_root.join("src")).unwrap();
        std::fs::write(workspace_root.join("src/main.rs"), "fn main() {}").unwrap();
        let repo = Arc::new(FakeWorkspaceRepo::default());
        *repo.workspace.write().await = Some(row("u1", "u1/ws_1"));
        let resolver = WorkspaceResolver::new(senmo.path().to_path_buf(), repo);

        let resolved = resolver
            .resolve_for_user("u1", "ws_1", "src/main.rs", WorkspaceResolveMode::Existing)
            .await
            .unwrap();

        let senmo_display = senmo.path().to_string_lossy();
        assert_eq!(resolved.relative_path, "src/main.rs");
        assert!(!resolved.relative_path.contains(senmo_display.as_ref()));
        assert!(resolved.absolute_path.starts_with(senmo.path().canonicalize().unwrap()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escape() {
        let senmo = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let workspace_root = senmo.path().join("u1/ws_1");
        std::fs::create_dir_all(&workspace_root).unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), workspace_root.join("leak.txt")).unwrap();
        let repo = Arc::new(FakeWorkspaceRepo::default());
        *repo.workspace.write().await = Some(row("u1", "u1/ws_1"));
        let resolver = WorkspaceResolver::new(senmo.path().to_path_buf(), repo);

        let error = resolver
            .resolve_for_user("u1", "ws_1", "leak.txt", WorkspaceResolveMode::Existing)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("escaped"));
    }
}
