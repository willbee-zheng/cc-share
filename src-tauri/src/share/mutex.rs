//! 供应者互斥与安全
//!
//! 监测本地用户是否正在使用某个 Provider，
//! 当检测到本地使用时标记为 Busy，暂停云端任务派发。
//! 任务完成后添加随机冷却延迟，模拟人类使用习惯。

use std::collections::HashSet;
use std::sync::Mutex;

/// 互斥检查器
///
/// 追踪哪些 Provider 正在被本地用户使用。
/// 当检测到本地使用时，标记节点为 Busy，
/// 云端不再派发该 Provider 的任务。
pub struct MutexChecker {
    /// 当前正在被本地使用的 Provider ID 集合
    busy_providers: Mutex<HashSet<String>>,
}

impl MutexChecker {
    /// 创建互斥检查器
    pub fn new() -> Self {
        Self {
            busy_providers: Mutex::new(HashSet::new()),
        }
    }

    /// 检查指定 Provider 是否正忙
    pub fn is_busy(&self, provider_id: &str) -> bool {
        self.busy_providers
            .lock()
            .expect("busy_providers mutex poisoned")
            .contains(provider_id)
    }

    /// 设置指定 Provider 的忙碌状态
    pub fn set_busy(&self, provider_id: &str, busy: bool) {
        let mut set = self
            .busy_providers
            .lock()
            .expect("busy_providers mutex poisoned");
        if busy {
            set.insert(provider_id.to_string());
        } else {
            set.remove(provider_id);
        }
    }

    /// 获取所有忙碌的 Provider ID
    pub fn busy_providers(&self) -> Vec<String> {
        self.busy_providers
            .lock()
            .expect("busy_providers mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }

    /// 清除所有忙碌状态
    pub fn clear_all(&self) {
        self.busy_providers
            .lock()
            .expect("busy_providers mutex poisoned")
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutex_checker_new() {
        let checker = MutexChecker::new();
        assert!(!checker.is_busy("provider-1"));
    }

    #[test]
    fn test_mutex_checker_set_busy() {
        let checker = MutexChecker::new();
        checker.set_busy("provider-1", true);
        assert!(checker.is_busy("provider-1"));
        assert!(!checker.is_busy("provider-2"));
    }

    #[test]
    fn test_mutex_checker_clear_busy() {
        let checker = MutexChecker::new();
        checker.set_busy("provider-1", true);
        checker.set_busy("provider-1", false);
        assert!(!checker.is_busy("provider-1"));
    }

    #[test]
    fn test_mutex_checker_busy_providers() {
        let checker = MutexChecker::new();
        checker.set_busy("p1", true);
        checker.set_busy("p2", true);
        let mut providers = checker.busy_providers();
        providers.sort();
        assert_eq!(providers, vec!["p1", "p2"]);
    }

    #[test]
    fn test_mutex_checker_clear_all() {
        let checker = MutexChecker::new();
        checker.set_busy("p1", true);
        checker.set_busy("p2", true);
        checker.clear_all();
        assert!(!checker.is_busy("p1"));
        assert!(!checker.is_busy("p2"));
    }
}