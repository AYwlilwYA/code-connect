package com.sample;

// 认证异常类
public class AuthException extends Exception {
    private String code;

    public AuthException(String message, String code) {
        super(message);
        this.code = code;
    }

    public String getCode() {
        return code;
    }
}
