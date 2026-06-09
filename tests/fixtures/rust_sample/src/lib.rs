use serde::{Deserialize, Serialize};

/// 用户数据结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub active: bool,
}

/// 认证服务 trait
pub trait AuthService {
    /// 验证用户凭证
    fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError>;
    /// 检查用户是否活跃
    fn is_active(&self, user: &User) -> bool;
}

/// 认证错误枚举
#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
    AccountLocked,
    NetworkError(String),
}

/// 默认认证服务实现
pub struct DefaultAuthService;

impl DefaultAuthService {
    /// 创建新的认证服务实例
    pub fn new() -> Self {
        Self
    }

    pub fn validate_token(&self, token: &str) -> bool {
        !token.is_empty()
    }
}

impl AuthService for DefaultAuthService {
    fn authenticate(&self, username: &str, password: &str) -> Result<User, AuthError> {
        if username.is_empty() || password.is_empty() {
            return Err(AuthError::InvalidCredentials);
        }
        Ok(User {
            id: 1,
            name: username.to_string(),
            email: format!("{}@example.com", username),
            active: true,
        })
    }

    fn is_active(&self, user: &User) -> bool {
        user.active
    }
}

/// 用户仓库接口
pub trait UserRepository {
    fn find_by_id(&self, id: u64) -> Option<User>;
    fn save(&self, user: &User) -> Result<(), String>;
}

/// 计算用户活跃度分数
pub fn calculate_activity_score(user: &User, login_count: u64) -> f64 {
    let base = if user.active { 1.0 } else { 0.0 };
    let factor = (login_count as f64).ln() + 1.0;
    base * factor
}

/// 宏：快速创建默认用户
#[macro_export]
macro_rules! default_user {
    ($name:expr) => {
        $crate::User {
            id: 0,
            name: $name.to_string(),
            email: format!("{}@example.com", $name),
            active: true,
        }
    };
}

/// 辅助函数
fn internal_helper() -> String {
    "helper".to_string()
}

/// 模块常量
pub const MAX_LOGIN_ATTEMPTS: u32 = 5;

/// 类型别名
pub type UserId = u64;
