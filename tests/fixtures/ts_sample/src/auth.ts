// 认证服务接口
export interface AuthService {
  // 验证用户凭证
  authenticate(username: string, password: string): Promise<User>;
  // 检查用户是否活跃
  isActive(user: User): boolean;
}

// 用户数据结构
export interface UserData {
  id: number;
  name: string;
  email: string;
  active: boolean;
}

// 用户实体类
export class User implements UserData {
  id: number;
  name: string;
  email: string;
  active: boolean;

  constructor(id: number, name: string, email: string) {
    this.id = id;
    this.name = name;
    this.email = email;
    this.active = true;
  }

  // 获取用户显示名称
  getDisplayName(): string {
    return `${this.name} <${this.email}>`;
  }

  // 停用用户
  deactivate(): void {
    this.active = false;
  }
}

// 默认认证服务实现
export class DefaultAuthService implements AuthService {
  private loginAttempts: Map<string, number> = new Map();

  async authenticate(username: string, password: string): Promise<User> {
    if (!username || !password) {
      throw new AuthError("用户名或密码为空", "INVALID_CREDENTIALS");
    }

    const attempts = this.loginAttempts.get(username) || 0;
    if (attempts >= MAX_LOGIN_ATTEMPTS) {
      throw new AuthError("账户已锁定", "ACCOUNT_LOCKED");
    }

    const user = new User(Date.now(), username, `${username}@example.com`);
    this.loginAttempts.set(username, 0);
    return user;
  }

  isActive(user: User): boolean {
    return user.active;
  }

  // 验证 token 是否有效
  validateToken(token: string): boolean {
    return token.length > 0;
  }

  // 记录登录失败
  recordFailedAttempt(username: string): void {
    const current = this.loginAttempts.get(username) || 0;
    this.loginAttempts.set(username, current + 1);
  }
}

// 认证错误类
export class AuthError extends Error {
  code: string;

  constructor(message: string, code: string) {
    super(message);
    this.name = "AuthError";
    this.code = code;
  }
}

// 计算用户活跃度分数
export function calculateActivityScore(user: User, loginCount: number): number {
  const base = user.active ? 1.0 : 0.0;
  const factor = Math.log(loginCount + 1) + 1;
  return base * factor;
}

// 用户仓库接口
export interface UserRepository {
  findById(id: number): User | null;
  save(user: User): void;
}

// 模块常量
export const MAX_LOGIN_ATTEMPTS = 5;

// 模块级辅助函数
function internalHelper(): string {
  return "helper";
}
