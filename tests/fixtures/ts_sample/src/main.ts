// 入口点：使用认证模块

import {
  AuthService,
  DefaultAuthService,
  UserRepository,
  User,
  calculateActivityScore,
  MAX_LOGIN_ATTEMPTS,
} from "./auth";

// 内存用户仓库实现
class InMemoryRepository implements UserRepository {
  private users: Map<number, User> = new Map();

  findById(id: number): User | null {
    return this.users.get(id) || null;
  }

  save(user: User): void {
    this.users.set(user.id, user);
    console.log(`保存用户: ${user.name}`);
  }
}

async function main(): Promise<void> {
  const authService: AuthService = new DefaultAuthService();

  try {
    // 调用 authenticate
    const user = await authService.authenticate("admin", "secret");

    // 调用 isActive
    if (authService.isActive(user)) {
      // 调用 calculateActivityScore
      const score = calculateActivityScore(user, 10);
      console.log(`用户 ${user.getDisplayName()} 活跃度: ${score.toFixed(2)}`);

      // 调用 validateToken
      const valid = (authService as DefaultAuthService).validateToken("abc123");
      console.log(`Token 有效: ${valid}`);

      // 使用常量
      console.log(`最大登录尝试次数: ${MAX_LOGIN_ATTEMPTS}`);
    }

    // 使用 UserRepository
    const repo = new InMemoryRepository();
    repo.save(user);
  } catch (e) {
    if (e instanceof Error) {
      console.error(`认证失败: ${e.message}`);
    }
  }
}

main();
