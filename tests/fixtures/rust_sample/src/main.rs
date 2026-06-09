use rust_sample::{
    AuthService, DefaultAuthService, UserRepository,
    calculate_activity_score, MAX_LOGIN_ATTEMPTS,
};

struct InMemoryRepository {
    // 内部实现
}

impl InMemoryRepository {
    fn new() -> Self {
        Self {}
    }
}

impl UserRepository for InMemoryRepository {
    fn find_by_id(&self, id: u64) -> Option<rust_sample::User> {
        if id == 0 { None } else { Some(rust_sample::User { id, name: "test".into(), email: "test@test.com".into(), active: true }) }
    }

    fn save(&self, user: &rust_sample::User) -> Result<(), String> {
        println!("保存用户: {}", user.name);
        Ok(())
    }
}

fn main() {
    let auth_service = DefaultAuthService::new();

    // 调用 authenticate
    match auth_service.authenticate("admin", "secret") {
        Ok(user) => {
            // 调用 is_active
            if auth_service.is_active(&user) {
                // 调用 calculate_activity_score
                let score = calculate_activity_score(&user, 10);
                println!("用户 {} 活跃度: {:.2}", user.name, score);

                // 调用 validate_token
                let valid = auth_service.validate_token("abc123");
                println!("Token 有效: {}", valid);

                // 使用常量
                println!("最大登录尝试次数: {}", MAX_LOGIN_ATTEMPTS);
            }

            // 使用 UserRepository
            let repo = InMemoryRepository::new();
            let _ = repo.save(&user);
        }
        Err(e) => eprintln!("认证失败: {:?}", e),
    }
}
