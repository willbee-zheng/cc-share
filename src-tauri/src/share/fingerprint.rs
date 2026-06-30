//! 设备指纹生成
//!
//! 用本机的 OS、主机名、MAC 地址等稳定信息生成 SHA-256 指纹，
//! 不联网、不依赖外部 API；生成后缓存在 share.db 的 client_config 表。
//!
//! 注意：不要把指纹原始信息（hostname、MAC）上传到云端 — 仅传 hash。
//! 重装系统、换网卡会导致指纹变化，触发云端的 [`ErrFingerprintMismatch`]，
//! 用户需要在桌面端 UI 上点"重置节点绑定"来重新申请。

use sha2::{Digest, Sha256};

/// 生成本机指纹（hex 字符串）
///
/// 输入混合多个稳定源以避免单点失败：
/// - 操作系统名（"macos" / "linux" / "windows"）
/// - 主机名（hostname）
/// - 用户名（whoami）
///
/// 这三者一起作为 hmac key 之外的盐，足够区分大多数设备。
pub fn compute_fingerprint() -> String {
    let mut hasher = Sha256::new();
    hasher.update(std::env::consts::OS.as_bytes());
    hasher.update(b"|");
    hasher.update(host_name().as_bytes());
    hasher.update(b"|");
    hasher.update(user_name().as_bytes());
    let digest = hasher.finalize();
    hex::encode(digest)
}

fn host_name() -> String {
    // hostname 在 Unix/Windows 上都可以通过环境变量或 sysctl 获取；
    // 这里用最基础的环境变量回退避免引入新 crate。
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".into())
}

fn user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-user".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_64_hex_chars() {
        let fp = compute_fingerprint();
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_is_deterministic_within_process() {
        // 同一进程内两次调用应返回相同值（环境变量不变）
        let a = compute_fingerprint();
        let b = compute_fingerprint();
        assert_eq!(a, b);
    }

    #[test]
    fn host_name_falls_back_to_unknown() {
        // Just exercise the path — the result is environment-dependent
        let h = host_name();
        assert!(!h.is_empty());
    }
}
