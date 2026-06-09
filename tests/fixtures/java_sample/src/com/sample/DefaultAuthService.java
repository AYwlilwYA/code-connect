package com.sample;

import java.util.HashMap;
import java.util.Map;

// 默认认证服务实现
public class DefaultAuthService implements AuthService {
    private Map<String, Integer> loginAttempts = new HashMap<>();
    private static final int MAX_LOGIN_ATTEMPTS = 5;

    @Override
    public User authenticate(String username, String password) throws AuthException {
        if (username == null || username.isEmpty() || password == null || password.isEmpty()) {
            throw new AuthException("用户名或密码为空", "INVALID_CREDENTIALS");
        }

        int attempts = loginAttempts.getOrDefault(username, 0);
        if (attempts >= MAX_LOGIN_ATTEMPTS) {
            throw new AuthException("账户已锁定", "ACCOUNT_LOCKED");
        }

        User user = new User(System.currentTimeMillis(), username, username + "@example.com");
        loginAttempts.put(username, 0);
        return user;
    }

    @Override
    public boolean isActive(User user) {
        return user.isActive();
    }

    // 验证 token 是否有效
    public boolean validateToken(String token) {
        return token != null && !token.isEmpty();
    }

    // 记录登录失败
    public void recordFailedAttempt(String username) {
        int current = loginAttempts.getOrDefault(username, 0);
        loginAttempts.put(username, current + 1);
    }

    // 计算用户活跃度分数
    public static double calculateActivityScore(User user, long loginCount) {
        double base = user.isActive() ? 1.0 : 0.0;
        double factor = Math.log(loginCount + 1) + 1.0;
        return base * factor;
    }
}
