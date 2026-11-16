use std::collections::BTreeMap;
use std::sync::Mutex;

use ayu_core::{
    AccountId, CompactMessage, CoreError, DialogId, MessageFlags, MessageRepository, MsgId, PeerId,
    UserId, VirtualizedHistoryWindow,
};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use smallvec::SmallVec;

struct BenchRepo {
    rows: Mutex<BTreeMap<i32, CompactMessage>>,
}

impl BenchRepo {
    fn new(count: i32) -> Self {
        let mut map = BTreeMap::new();
        for i in 1..=count {
            map.insert(
                i,
                CompactMessage {
                    dialog_id: DialogId(1),
                    sender_id: PeerId::from_user(UserId(1)),
                    grouped_id: 0,
                    text: format!("bench message payload {i}").into_boxed_str(),
                    author_name: None,
                    media: None,
                    entities: SmallVec::new(),
                    id: MsgId(i),
                    reply_to_msg_id: MsgId(0),
                    date_unix: 1_700_000_000 + i,
                    edit_date_unix: 0,
                    views: 0,
                    flags: MessageFlags::empty(),
                },
            );
        }
        Self {
            rows: Mutex::new(map),
        }
    }
}

impl MessageRepository for BenchRepo {
    fn upsert_message(
        &self,
        _account_id: AccountId,
        _message: &CompactMessage,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    fn upsert_batch(
        &self,
        _account_id: AccountId,
        messages: &[CompactMessage],
    ) -> Result<usize, CoreError> {
        Ok(messages.len())
    }

    fn load_slice_around(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
        center_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        let guard = self
            .rows
            .lock()
            .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
        let start = center_msg_id.0.saturating_sub((limit / 2) as i32);
        Ok(guard
            .range(start..)
            .take(limit)
            .map(|(_, v)| v.clone())
            .collect())
    }

    fn load_older_than(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
        before_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        let guard = self
            .rows
            .lock()
            .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
        let mut slice: Vec<CompactMessage> = guard
            .range(..before_msg_id.0)
            .rev()
            .take(limit)
            .map(|(_, v)| v.clone())
            .collect();
        slice.reverse();
        Ok(slice)
    }

    fn load_newer_than(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
        after_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        let guard = self
            .rows
            .lock()
            .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
        Ok(guard
            .range(after_msg_id.0.saturating_add(1)..)
            .take(limit)
            .map(|(_, v)| v.clone())
            .collect())
    }

    fn delete_messages(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
        _msg_ids: &[MsgId],
    ) -> Result<usize, CoreError> {
        Ok(0)
    }

    fn delete_messages_up_to(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
        _max_msg_id: MsgId,
    ) -> Result<usize, CoreError> {
        Ok(0)
    }

    fn search_dialog_messages(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        Ok(Vec::new())
    }

    fn dialog_message_count(
        &self,
        _account_id: AccountId,
        _dialog_id: DialogId,
    ) -> Result<usize, CoreError> {
        Ok(0)
    }
}

fn bench_virtualized_window_scroll(c: &mut Criterion) {
    let repo = BenchRepo::new(2_000);
    if let Ok(mut window) = VirtualizedHistoryWindow::new(repo, AccountId(1), DialogId(1), 100) {
        let _ = window.jump_to(MsgId(1_000));
        c.bench_function("virtualized_window_scroll_20", |b| {
            b.iter(|| {
                let _ = window.scroll_up(black_box(20));
                let _ = window.scroll_down(black_box(20));
            });
        });
    }
}

criterion_group!(benches, bench_virtualized_window_scroll);
criterion_main!(benches);
