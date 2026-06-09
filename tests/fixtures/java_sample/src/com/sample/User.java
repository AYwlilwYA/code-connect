package com.sample;

// 用户实体类
public class User {
    private long id;
    private String name;
    private String email;
    private boolean active;

    // 构造器
    public User(long id, String name, String email) {
        this.id = id;
        this.name = name;
        this.email = email;
        this.active = true;
    }

    // 获取用户显示名称
    public String getDisplayName() {
        return this.name + " <" + this.email + ">";
    }

    // 停用用户
    public void deactivate() {
        this.active = false;
    }

    // getter 方法
    public long getId() { return id; }
    public void setId(long id) { this.id = id; }

    public String getName() { return name; }
    public void setName(String name) { this.name = name; }

    public String getEmail() { return email; }
    public void setEmail(String email) { this.email = email; }

    public boolean isActive() { return active; }
    public void setActive(boolean active) { this.active = active; }
}
