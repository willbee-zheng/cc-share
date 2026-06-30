//! 拟人化延迟引擎
//!
//! 模拟人类使用行为，降低封号风险：
//! - 请求间随机延迟
//! - 活跃时段控制
//! - 打字速度模拟（Web 模式）

use chrono::Timelike;
use rand::Rng;
use std::time::Duration;

/// 拟人化配置
#[derive(Debug, Clone)]
pub struct HumanizerConfig {
    /// 请求间最小延迟（秒）
    pub min_cooldown_secs: u64,
    /// 请求间最大延迟（秒）
    pub max_cooldown_secs: u64,
    /// 是否启用活跃时段限制
    pub enable_active_hours: bool,
    /// 活跃时段开始（小时，0-23）
    pub active_hours_start: u8,
    /// 活跃时段结束（小时，0-23）
    pub active_hours_end: u8,
}

impl Default for HumanizerConfig {
    fn default() -> Self {
        Self {
            min_cooldown_secs: 30,
            max_cooldown_secs: 120,
            enable_active_hours: true,
            active_hours_start: 8,
            active_hours_end: 23,
        }
    }
}

/// 拟人化延迟引擎
///
/// 提供请求间的随机延迟和活跃时段判断，
/// 模拟人类使用模式以降低封号风险。
pub struct Humanizer {
    config: HumanizerConfig,
}

impl Humanizer {
    /// 创建拟人化引擎
    pub fn new(config: HumanizerConfig) -> Self {
        Self { config }
    }

    /// 使用默认配置创建
    pub fn with_defaults() -> Self {
        Self::new(HumanizerConfig::default())
    }

    /// 计算下一个请求的随机延迟
    ///
    /// 在 `[min_cooldown_secs, max_cooldown_secs]` 范围内
    /// 生成均匀分布的随机延迟。
    pub fn next_cooldown(&self) -> Duration {
        let mut rng = rand::thread_rng();
        let delay_secs = rng.gen_range(self.config.min_cooldown_secs..=self.config.max_cooldown_secs);
        Duration::from_secs(delay_secs)
    }

    /// 检查当前是否在活跃时段内
    ///
    /// 支持跨午夜的时段（如 22:00-06:00）。
    /// 如果 `enable_active_hours` 为 false，始终返回 true。
    pub fn is_active_hours(&self) -> bool {
        if !self.config.enable_active_hours {
            return true;
        }

        let now = chrono::Local::now().hour() as u8;
        self.is_hour_in_range(now)
    }

    /// 判断指定小时是否在活跃时段内
    fn is_hour_in_range(&self, hour: u8) -> bool {
        let start = self.config.active_hours_start;
        let end = self.config.active_hours_end;

        if start <= end {
            // 不跨午夜：如 08:00-23:00
            hour >= start && hour < end
        } else {
            // 跨午夜：如 22:00-06:00
            hour >= start || hour < end
        }
    }

    /// 计算到下一个活跃时段的等待时间
    ///
    /// 如果当前在活跃时段内，返回 0。
    pub fn time_until_active(&self) -> Duration {
        if self.is_active_hours() {
            return Duration::ZERO;
        }

        // 计算距离下一个活跃开始的小时数
        let now = chrono::Local::now();
        let current_hour = now.hour() as u8;
        let start = self.config.active_hours_start;

        let hours_until = if current_hour < start {
            start - current_hour
        } else {
            24 - current_hour + start
        };

        Duration::from_secs(hours_until as u64 * 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_humanizer_default_config() {
        let config = HumanizerConfig::default();
        assert_eq!(config.min_cooldown_secs, 30);
        assert_eq!(config.max_cooldown_secs, 120);
        assert!(config.enable_active_hours);
        assert_eq!(config.active_hours_start, 8);
        assert_eq!(config.active_hours_end, 23);
    }

    #[test]
    fn test_humanizer_cooldown_range() {
        let humanizer = Humanizer::with_defaults();
        for _ in 0..100 {
            let cooldown = humanizer.next_cooldown();
            assert!(cooldown.as_secs() >= 30);
            assert!(cooldown.as_secs() <= 120);
        }
    }

    #[test]
    fn test_active_hours_no_overlap() {
        let config = HumanizerConfig {
            enable_active_hours: true,
            active_hours_start: 8,
            active_hours_end: 23,
            ..Default::default()
        };
        let humanizer = Humanizer::new(config);

        assert!(humanizer.is_hour_in_range(8));
        assert!(humanizer.is_hour_in_range(12));
        assert!(humanizer.is_hour_in_range(22));
        assert!(!humanizer.is_hour_in_range(23));
        assert!(!humanizer.is_hour_in_range(0));
        assert!(!humanizer.is_hour_in_range(7));
    }

    #[test]
    fn test_active_hours_cross_midnight() {
        let config = HumanizerConfig {
            enable_active_hours: true,
            active_hours_start: 22,
            active_hours_end: 6,
            ..Default::default()
        };
        let humanizer = Humanizer::new(config);

        assert!(humanizer.is_hour_in_range(22));
        assert!(humanizer.is_hour_in_range(23));
        assert!(humanizer.is_hour_in_range(0));
        assert!(humanizer.is_hour_in_range(5));
        assert!(!humanizer.is_hour_in_range(6));
        assert!(!humanizer.is_hour_in_range(12));
    }

    #[test]
    fn test_active_hours_disabled() {
        let config = HumanizerConfig {
            enable_active_hours: false,
            ..Default::default()
        };
        let humanizer = Humanizer::new(config);
        assert!(humanizer.is_active_hours());
    }
}