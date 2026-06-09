package com.sample;

// 用户仓库接口
public interface UserRepository {
    // 按 ID 查找用户
    User findById(long id);
    // 保存用户
    void save(User user);
}
