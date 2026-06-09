package com.sample;

// 程序入口点：演示认证模块的使用
public class Main {
    // 内存用户仓库实现
    static class InMemoryRepository implements UserRepository {
        private java.util.Map<Long, User> users = new java.util.HashMap<>();

        @Override
        public User findById(long id) {
            return users.get(id);
        }

        @Override
        public void save(User user) {
            users.put(user.getId(), user);
            System.out.println("保存用户: " + user.getName());
        }
    }

    public static void main(String[] args) {
        DefaultAuthService authService = new DefaultAuthService();

        try {
            // 调用 authenticate
            User user = authService.authenticate("admin", "secret");

            // 调用 isActive
            if (authService.isActive(user)) {
                // 调用 calculateActivityScore
                double score = DefaultAuthService.calculateActivityScore(user, 10);
                System.out.printf("用户 %s 活跃度: %.2f%n", user.getDisplayName(), score);

                // 调用 validateToken
                boolean valid = authService.validateToken("abc123");
                System.out.println("Token 有效: " + valid);
            }

            // 使用 UserRepository
            InMemoryRepository repo = new InMemoryRepository();
            repo.save(user);
        } catch (AuthException e) {
            System.err.println("认证失败: " + e.getMessage());
        }
    }
}
