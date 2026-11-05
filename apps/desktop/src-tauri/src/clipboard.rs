//! 宿主侧剪贴板回声抑制。
//!
//! `display.clipboard.set` 把文本写进窗口选区后，合成器会把新的选区内容当成
//! 一次真实变化，原样回推一条 `display.clipboard.changed`。若照单全收，宿主
//! 会把自己刚下发的文本再写回自己的剪贴板：轻则无意义地重建选区（宿主上别的
//! 应用正在读的那份会被打断），重则两端互相回推形成环。
//!
//! RFC 0004 的建议是记住本端最后一次下发的文本，收到相同文本的上行时不再转
//! 发。这里记一个短队列而不是单个值：连续两次下发时，两条回声的到达顺序没有
//! 保证，单值会漏掉一条。

use std::collections::VecDeque;

/// 待回声队列上限。合成器正常会回推一条；记太多只在对端根本不回声时才堆着。
const MAX_PENDING: usize = 8;

#[derive(Debug, Default)]
pub struct ClipboardEcho {
    pending: VecDeque<String>,
}

impl ClipboardEcho {
    /// 记下一次下发，供随后到达的 `display.clipboard.changed` 比对。
    pub fn note_set(&mut self, text: &str) {
        if self.pending.len() == MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(text.to_owned());
    }

    /// 下发失败时撤销登记：否则下一次真实的相同上行会被当成回声吞掉。
    pub fn cancel(&mut self, text: &str) {
        if let Some(index) = self.pending.iter().position(|entry| entry == text) {
            self.pending.remove(index);
        }
    }

    /// 返回 true 表示这条上行应当转给宿主剪贴板。
    pub fn accepts(&mut self, text: &str) -> bool {
        if let Some(index) = self.pending.iter().position(|entry| entry == text) {
            self.pending.remove(index);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swallows_exactly_one_echo_per_set() {
        let mut echo = ClipboardEcho::default();
        echo.note_set("hello");
        // 合成器回推：丢弃。
        assert!(!echo.accepts("hello"));
        // 容器里随后真的复制了同样的文本：必须转发，否则宿主剪贴板停在旧值。
        assert!(echo.accepts("hello"));
    }

    #[test]
    fn forwards_text_it_did_not_send() {
        let mut echo = ClipboardEcho::default();
        echo.note_set("hello");
        assert!(echo.accepts("world"));
        // 没有匹配到的下发登记还在，稍后到达的回声仍要被吞掉。
        assert!(!echo.accepts("hello"));
    }

    #[test]
    fn cancel_releases_the_slot() {
        let mut echo = ClipboardEcho::default();
        echo.note_set("hello");
        echo.cancel("hello");
        assert!(echo.accepts("hello"));
    }

    #[test]
    fn out_of_order_echoes_both_match() {
        let mut echo = ClipboardEcho::default();
        echo.note_set("first");
        echo.note_set("second");
        assert!(!echo.accepts("second"));
        assert!(!echo.accepts("first"));
        assert!(echo.accepts("first"));
    }

    #[test]
    fn bounds_the_queue() {
        let mut echo = ClipboardEcho::default();
        for index in 0..(MAX_PENDING + 1) {
            echo.note_set(&format!("text-{index}"));
        }
        // 最老的一条被挤掉，不再被当成回声。
        assert!(echo.accepts("text-0"));
        assert!(!echo.accepts(&format!("text-{MAX_PENDING}")));
    }
}
