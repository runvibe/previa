use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Root,
    Admin,
    Editor,
    Operator,
    Viewer,
    Anonymous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ManageUsers,
    ManageApiTokens,
    ReadProjects,
    WriteProjects,
    RunExecutions,
    DeleteHistory,
    ReadRunners,
    ManageRunners,
    ProxyRequests,
    UseMcp,
}

impl Role {
    pub fn allows(self, permission: Permission) -> bool {
        match self {
            Self::Root | Self::Anonymous => true,
            Self::Admin => true,
            Self::Editor => matches!(
                permission,
                Permission::ReadProjects
                    | Permission::WriteProjects
                    | Permission::RunExecutions
                    | Permission::DeleteHistory
                    | Permission::ReadRunners
                    | Permission::ProxyRequests
                    | Permission::UseMcp
            ),
            Self::Operator => matches!(
                permission,
                Permission::ReadProjects
                    | Permission::RunExecutions
                    | Permission::ReadRunners
                    | Permission::ProxyRequests
                    | Permission::UseMcp
            ),
            Self::Viewer => matches!(
                permission,
                Permission::ReadProjects | Permission::ReadRunners | Permission::UseMcp
            ),
        }
    }

    pub fn can_create_role(self, target: Self) -> bool {
        match self {
            Self::Root => true,
            Self::Admin => !matches!(target, Self::Root | Self::Anonymous),
            _ => false,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Root => "root",
            Self::Admin => "admin",
            Self::Editor => "editor",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
            Self::Anonymous => "anonymous",
        })
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "root" => Ok(Self::Root),
            "admin" => Ok(Self::Admin),
            "editor" => Ok(Self::Editor),
            "operator" => Ok(Self::Operator),
            "viewer" => Ok(Self::Viewer),
            "anonymous" => Ok(Self::Anonymous),
            _ => Err(format!("invalid role '{value}'")),
        }
    }
}
