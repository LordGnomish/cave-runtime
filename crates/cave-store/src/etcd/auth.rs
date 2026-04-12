//! etcd Auth service — users, roles, permissions, token auth.

use std::{
    collections::HashMap,
    sync::Arc,
};

use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use super::proto::{
    authpb,
    etcdserverpb::{
        auth_server::Auth, AuthDisableRequest, AuthDisableResponse, AuthEnableRequest,
        AuthEnableResponse, AuthStatusRequest, AuthStatusResponse, AuthUserAddRequest,
        AuthUserAddResponse, AuthUserChangePasswordRequest, AuthUserChangePasswordResponse,
        AuthUserDeleteRequest, AuthUserDeleteResponse, AuthUserGetRequest, AuthUserGetResponse,
        AuthUserGrantRoleRequest, AuthUserGrantRoleResponse, AuthUserListRequest,
        AuthUserListResponse, AuthUserRevokeRoleRequest, AuthUserRevokeRoleResponse,
        AuthenticateRequest, AuthenticateResponse, AuthRoleAddRequest, AuthRoleAddResponse,
        AuthRoleDeleteRequest, AuthRoleDeleteResponse, AuthRoleGetRequest, AuthRoleGetResponse,
        AuthRoleGrantPermissionRequest, AuthRoleGrantPermissionResponse, AuthRoleListRequest,
        AuthRoleListResponse, AuthRoleRevokePermissionRequest, AuthRoleRevokePermissionResponse,
        ResponseHeader,
    },
};

// ─── Internal types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct AuthUser {
    name: String,
    hashed_password: String, // hex(sha256(salt + password))
    salt: String,
    roles: Vec<String>,
}

#[derive(Debug, Clone)]
struct AuthRole {
    name: String,
    permissions: Vec<authpb::Permission>,
}

#[derive(Debug, Default)]
struct AuthState {
    enabled: bool,
    auth_revision: u64,
    users: HashMap<String, AuthUser>,
    roles: HashMap<String, AuthRole>,
    /// token → username
    tokens: HashMap<String, String>,
}

impl AuthState {
    fn hash_password(password: &str) -> (String, String) {
        let salt = Uuid::new_v4().to_string();
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        hasher.update(password.as_bytes());
        let hash = hex::encode(hasher.finalize());
        (salt, hash)
    }

    fn verify_password(user: &AuthUser, password: &str) -> bool {
        let mut hasher = Sha256::new();
        hasher.update(user.salt.as_bytes());
        hasher.update(password.as_bytes());
        let hash = hex::encode(hasher.finalize());
        hash == user.hashed_password
    }
}

pub struct AuthServer {
    state: Arc<RwLock<AuthState>>,
}

impl AuthServer {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(AuthState::default())),
        }
    }

    fn header(&self) -> ResponseHeader {
        ResponseHeader { cluster_id: 1, member_id: 1, revision: 0, raft_term: 1 }
    }
}

impl Default for AuthServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl Auth for AuthServer {
    async fn auth_enable(&self, _: Request<AuthEnableRequest>) -> Result<Response<AuthEnableResponse>, Status> {
        let mut s = self.state.write();
        s.enabled = true;
        s.auth_revision += 1;
        Ok(Response::new(AuthEnableResponse { header: Some(self.header()) }))
    }

    async fn auth_disable(&self, _: Request<AuthDisableRequest>) -> Result<Response<AuthDisableResponse>, Status> {
        let mut s = self.state.write();
        s.enabled = false;
        s.auth_revision += 1;
        Ok(Response::new(AuthDisableResponse { header: Some(self.header()) }))
    }

    async fn auth_status(&self, _: Request<AuthStatusRequest>) -> Result<Response<AuthStatusResponse>, Status> {
        let s = self.state.read();
        Ok(Response::new(AuthStatusResponse {
            header: Some(self.header()),
            enabled: s.enabled,
            auth_revision: s.auth_revision,
        }))
    }

    async fn authenticate(&self, req: Request<AuthenticateRequest>) -> Result<Response<AuthenticateResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        let user = s.users.get(&r.name)
            .ok_or_else(|| Status::not_found("user not found"))?
            .clone();
        if !AuthState::verify_password(&user, &r.password) {
            return Err(Status::unauthenticated("invalid password"));
        }
        let token = Uuid::new_v4().to_string();
        s.tokens.insert(token.clone(), r.name);
        Ok(Response::new(AuthenticateResponse { header: Some(self.header()), token }))
    }

    async fn user_add(&self, req: Request<AuthUserAddRequest>) -> Result<Response<AuthUserAddResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        if s.users.contains_key(&r.name) {
            return Err(Status::already_exists(format!("user {} already exists", r.name)));
        }
        let no_pw = r.options.as_ref().map(|o| o.no_password).unwrap_or(false);
        let (salt, hashed_password) = if no_pw || r.password.is_empty() {
            (String::new(), String::new())
        } else {
            AuthState::hash_password(&r.password)
        };
        s.users.insert(r.name.clone(), AuthUser {
            name: r.name,
            hashed_password,
            salt,
            roles: vec![],
        });
        s.auth_revision += 1;
        Ok(Response::new(AuthUserAddResponse { header: Some(self.header()) }))
    }

    async fn user_get(&self, req: Request<AuthUserGetRequest>) -> Result<Response<AuthUserGetResponse>, Status> {
        let r = req.into_inner();
        let s = self.state.read();
        let user = s.users.get(&r.name).ok_or_else(|| Status::not_found("user not found"))?;
        Ok(Response::new(AuthUserGetResponse {
            header: Some(self.header()),
            roles: user.roles.clone(),
        }))
    }

    async fn user_list(&self, _: Request<AuthUserListRequest>) -> Result<Response<AuthUserListResponse>, Status> {
        let s = self.state.read();
        Ok(Response::new(AuthUserListResponse {
            header: Some(self.header()),
            users: s.users.keys().cloned().collect(),
        }))
    }

    async fn user_delete(&self, req: Request<AuthUserDeleteRequest>) -> Result<Response<AuthUserDeleteResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        if s.users.remove(&r.name).is_none() {
            return Err(Status::not_found("user not found"));
        }
        s.auth_revision += 1;
        Ok(Response::new(AuthUserDeleteResponse { header: Some(self.header()) }))
    }

    async fn user_change_password(&self, req: Request<AuthUserChangePasswordRequest>) -> Result<Response<AuthUserChangePasswordResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        let user = s.users.get_mut(&r.name).ok_or_else(|| Status::not_found("user not found"))?;
        let (salt, hashed) = AuthState::hash_password(&r.password);
        user.salt = salt;
        user.hashed_password = hashed;
        s.auth_revision += 1;
        Ok(Response::new(AuthUserChangePasswordResponse { header: Some(self.header()) }))
    }

    async fn user_grant_role(&self, req: Request<AuthUserGrantRoleRequest>) -> Result<Response<AuthUserGrantRoleResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        if !s.roles.contains_key(&r.role) {
            return Err(Status::not_found(format!("role {} not found", r.role)));
        }
        let user = s.users.get_mut(&r.user).ok_or_else(|| Status::not_found("user not found"))?;
        if !user.roles.contains(&r.role) {
            user.roles.push(r.role);
        }
        s.auth_revision += 1;
        Ok(Response::new(AuthUserGrantRoleResponse { header: Some(self.header()) }))
    }

    async fn user_revoke_role(&self, req: Request<AuthUserRevokeRoleRequest>) -> Result<Response<AuthUserRevokeRoleResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        let user = s.users.get_mut(&r.name).ok_or_else(|| Status::not_found("user not found"))?;
        user.roles.retain(|role| role != &r.role);
        s.auth_revision += 1;
        Ok(Response::new(AuthUserRevokeRoleResponse { header: Some(self.header()) }))
    }

    async fn role_add(&self, req: Request<AuthRoleAddRequest>) -> Result<Response<AuthRoleAddResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        if s.roles.contains_key(&r.name) {
            return Err(Status::already_exists(format!("role {} already exists", r.name)));
        }
        s.roles.insert(r.name.clone(), AuthRole { name: r.name, permissions: vec![] });
        s.auth_revision += 1;
        Ok(Response::new(AuthRoleAddResponse { header: Some(self.header()) }))
    }

    async fn role_get(&self, req: Request<AuthRoleGetRequest>) -> Result<Response<AuthRoleGetResponse>, Status> {
        let r = req.into_inner();
        let s = self.state.read();
        let role = s.roles.get(&r.role).ok_or_else(|| Status::not_found("role not found"))?;
        Ok(Response::new(AuthRoleGetResponse {
            header: Some(self.header()),
            perm: role.permissions.clone(),
        }))
    }

    async fn role_list(&self, _: Request<AuthRoleListRequest>) -> Result<Response<AuthRoleListResponse>, Status> {
        let s = self.state.read();
        Ok(Response::new(AuthRoleListResponse {
            header: Some(self.header()),
            roles: s.roles.keys().cloned().collect(),
        }))
    }

    async fn role_delete(&self, req: Request<AuthRoleDeleteRequest>) -> Result<Response<AuthRoleDeleteResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        if s.roles.remove(&r.role).is_none() {
            return Err(Status::not_found("role not found"));
        }
        // Remove role from all users
        for user in s.users.values_mut() {
            user.roles.retain(|role| role != &r.role);
        }
        s.auth_revision += 1;
        Ok(Response::new(AuthRoleDeleteResponse { header: Some(self.header()) }))
    }

    async fn role_grant_permission(&self, req: Request<AuthRoleGrantPermissionRequest>) -> Result<Response<AuthRoleGrantPermissionResponse>, Status> {
        let r = req.into_inner();
        let perm = r.perm.ok_or_else(|| Status::invalid_argument("permission required"))?;
        let mut s = self.state.write();
        let role = s.roles.get_mut(&r.name).ok_or_else(|| Status::not_found("role not found"))?;
        role.permissions.push(perm);
        s.auth_revision += 1;
        Ok(Response::new(AuthRoleGrantPermissionResponse { header: Some(self.header()) }))
    }

    async fn role_revoke_permission(&self, req: Request<AuthRoleRevokePermissionRequest>) -> Result<Response<AuthRoleRevokePermissionResponse>, Status> {
        let r = req.into_inner();
        let mut s = self.state.write();
        let role = s.roles.get_mut(&r.role).ok_or_else(|| Status::not_found("role not found"))?;
        role.permissions.retain(|p| p.key != r.key || p.range_end != r.range_end);
        s.auth_revision += 1;
        Ok(Response::new(AuthRoleRevokePermissionResponse { header: Some(self.header()) }))
    }
}
