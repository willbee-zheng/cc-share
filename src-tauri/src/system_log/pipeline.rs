//! 后台批量写入 task
//!
//! 从 mpsc rx 收日志条目，每 500 条或每 1 秒批量写入 SQLite。
//! 写入完成后调用 `on_flush(count)` 回调，供 `lib.rs` emit 事件给前端。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{interval, Instant};

use crate::database::ShareDb;
use crate::system_log::LogEntry;

const BATCH_MAX: usize = 500;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// 启动后台 batch writer。
///
/// 使用 `tauri::async_runtime::spawn` 而非 `tokio::spawn`，因为 `setup` 闭包
/// 可能在非 tokio 线程上执行，而 `tauri::async_runtime` 保证使用正确的 runtime handle。
pub fn spawn_batch_writer(
    db: Arc<ShareDb>,
    mut rx: UnboundedReceiver<LogEntry>,
    on_flush: Box<dyn Fn(usize) + Send + Sync>,
) {
    tauri::async_runtime::spawn(async move {
        let mut buffer: Vec<LogEntry> = Vec::with_capacity(BATCH_MAX);
        let mut ticker = interval(FLUSH_INTERVAL);
        // 跳过第一次立即触发（启动时空 buffer 没必要 flush）
        ticker.tick().await;

        let mut last_flush = Instant::now();

        loop {
            tokio::select! {
                // 收到一条日志就塞进 buffer；缓冲满则立即 flush
                msg = rx.recv() => {
                    match msg {
                        Some(entry) => {
                            buffer.push(entry);
                            if buffer.len() >= BATCH_MAX {
                                flush(&db, &mut buffer, &on_flush);
                                last_flush = Instant::now();
                            }
                        }
                        None => {
                            // channel 关闭：最后一次 flush 后退出
                            if !buffer.is_empty() {
                                flush(&db, &mut buffer, &on_flush);
                            }
                            log::debug!("system_log batch writer: channel closed, exiting");
                            return;
                        }
                    }
                }
                // 定时器触发：buffer 非空且距上次 flush 已超阈值
                _ = ticker.tick() => {
                    if !buffer.is_empty() && last_flush.elapsed() >= FLUSH_INTERVAL {
                        flush(&db, &mut buffer, &on_flush);
                        last_flush = Instant::now();
                    }
                }
            }
        }
    });
}

fn flush(db: &ShareDb, buffer: &mut Vec<LogEntry>, on_flush: &dyn Fn(usize)) {
    if buffer.is_empty() {
        return;
    }
    let to_write = std::mem::take(buffer);
    match db.insert_logs_batch(&to_write) {
        Ok(n) => on_flush(n),
        Err(e) => {
            log::error!("system_log batch insert 失败，丢弃 {} 条: {e}", to_write.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_log::LogFilter;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    #[tokio::test]
    async fn test_batch_writer_flushes_on_channel_close() {
        let db = Arc::new(ShareDb::memory().unwrap());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<LogEntry>();
        let flush_count = StdArc::new(AtomicUsize::new(0));
        let flush_for_cb = flush_count.clone();
        spawn_batch_writer(
            db.clone(),
            rx,
            Box::new(move |_n| {
                flush_for_cb.fetch_add(1, Ordering::SeqCst);
            }),
        );

        for i in 0..10 {
            tx.send(LogEntry {
                timestamp_ms: 1_700_000_000_000 + i,
                level: "info".into(),
                target: "test".into(),
                message: format!("msg {i}"),
            })
            .unwrap();
        }
        drop(tx); // 关闭 channel 触发最后一次 flush + 退出

        // 给 task 时间退出
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(flush_count.load(Ordering::SeqCst) >= 1, "应至少 flush 一次");
        let all = db.query_logs(&LogFilter::default()).unwrap();
        assert_eq!(all.len(), 10);
    }
}
