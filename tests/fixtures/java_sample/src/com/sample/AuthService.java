// 认证服务接口
public interface AuthService {
    // 验证用户凭证
    User authenticate(String username, String password) throws AuthException;
    // 检查用户是否活跃
    boolean isActive(User user);
}
