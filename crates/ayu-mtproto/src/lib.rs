pub mod error;
pub mod interceptor;
pub mod multiplexer;
pub mod protocol;
pub mod sync;
pub mod tokio_tcp;
pub mod transfer;

pub use error::MtprotoError;
pub use interceptor::{GhostPacketInterceptor, InterceptDecision};
pub use multiplexer::{AsyncTransport, BufferPool, DispatchOutcome, MtprotoMultiplexer, TransportFuture};
pub use protocol::{
    AuthKey, AuthKeyId, EncryptedEnvelope, MsgIdGenerator, ReconnectPolicy, SessionId, TlMethodTag,
};
pub use sync::{PendingOutgoingMessage, PtsCheckResult, UpdatesSynchronizer};
pub use tokio_tcp::{
    TokioTcpManager, TCP_EVENT_CONNECTED, TCP_EVENT_DISCONNECTED, TCP_EVENT_ERROR,
    TCP_EVENT_READY_READ,
};
pub use transfer::{
    compute_upload_part_size, FileTransferManager, TransferDirection, TransferProgress,
    TransferState, BIG_FILE_THRESHOLD, DEFAULT_MAX_INFLIGHT_BYTES, DOWNLOAD_PART_SIZE,
    UPLOAD_PART_SIZE_LARGE, UPLOAD_PART_SIZE_MEDIUM, UPLOAD_PART_SIZE_SMALL, UPLOAD_PART_SIZE_TINY,
    UPLOAD_PART_SIZE_XLARGE,
};

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ayu_core::{
        AccountId, DialogId, GhostModeConfig, GhostModeFlags, MsgId, NetworkTransport,
        SendWithoutSoundOption, SyncStateSnapshot,
    };
    use bytes::Bytes;

    use super::*;

    struct MockAsyncTransport {
        sent: Mutex<Vec<(AccountId, u16, Vec<u8>)>>,
        fail_next: Mutex<bool>,
    }

    impl MockAsyncTransport {
        fn new() -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                fail_next: Mutex::new(false),
            }
        }

        fn set_fail(&self, fail: bool) {
            if let Ok(mut guard) = self.fail_next.lock() {
                *guard = fail;
            }
        }

        fn recorded_count(&self) -> usize {
            self.sent.lock().map(|g| g.len()).unwrap_or(0)
        }
    }

    impl AsyncTransport for MockAsyncTransport {
        fn send_frame<'a>(
            &'a self,
            account_id: AccountId,
            dc_id: u16,
            frame: &'a [u8],
        ) -> TransportFuture<'a> {
            Box::pin(async move {
                let should_fail = self.fail_next.lock().map(|g| *g).unwrap_or(false);
                if should_fail {
                    return Err(MtprotoError::Disconnected {
                        dc_id,
                        reason: "simulated drop".to_owned(),
                    });
                }
                if let Ok(mut guard) = self.sent.lock() {
                    guard.push((account_id, dc_id, frame.to_vec()));
                }
                Ok(frame.len())
            })
        }

        fn is_connected(&self, _account_id: AccountId) -> bool {
            !self.fail_next.lock().map(|g| *g).unwrap_or(false)
        }
    }

    fn make_envelope(
        session_id: SessionId,
        msg_id: i64,
        method: TlMethodTag,
    ) -> EncryptedEnvelope {
        EncryptedEnvelope {
            dc_id: 2,
            auth_key_id: AuthKeyId([1; 8]),
            msg_key: [2; 16],
            session_id,
            msg_id,
            seq_no: 1,
            method,
            payload: Bytes::from_static(b"payload"),
        }
    }

    #[test]
    fn msg_id_generator_is_monotonic_and_divisible_by_four() {
        let gen = MsgIdGenerator::new();
        let mut prev = 0i64;
        for _ in 0..1_000 {
            let id = gen.next_msg_id();
            assert!(id > prev);
            assert_eq!(id % 4, 0);
            prev = id;
        }
    }

    #[test]
    fn reconnect_policy_bounds_delay() {
        let policy = ReconnectPolicy::default();
        let d0 = policy.delay_for_attempt(0, 7);
        let d10 = policy.delay_for_attempt(10, 123);
        assert!(d0.as_millis() >= 250);
        assert!(d10.as_millis() <= u128::from(policy.max_delay_ms));
    }

    #[tokio::test]
    async fn multiplexer_shares_buffer_pool_and_enforces_ghost_mode() -> Result<(), MtprotoError> {
        let transport = Arc::new(MockAsyncTransport::new());
        let mux = MtprotoMultiplexer::new(Arc::clone(&transport), 8);

        let acc1 = AccountId(1);
        let acc2 = AccountId(2);
        let ghost_on = GhostModeConfig::new(
            GhostModeFlags::GHOST_MODE_ACTIVE,
            SendWithoutSoundOption::InGhostMode,
        );
        let normal = GhostModeConfig::default();

        mux.register_session(acc1, 2, SessionId(11), ghost_on)?;
        mux.register_session(acc2, 4, SessionId(22), normal)?;

        // Account 1 with Ghost Mode suppresses ReadHistory without hitting network transport.
        let id1 = mux.next_msg_id();
        let res1 = mux
            .dispatch_envelope(
                acc1,
                make_envelope(SessionId(11), id1, TlMethodTag::ReadHistory),
            )
            .await?;
        assert_eq!(res1, DispatchOutcome::SuppressedByGhostMode);
        assert_eq!(mux.suppressed_count(acc1)?, 1);

        // Account 1 rewrites UpdateStatusOnline to offline when Ghost Mode is active.
        let id2 = mux.next_msg_id();
        let res2 = mux
            .dispatch_envelope(
                acc1,
                make_envelope(SessionId(11), id2, TlMethodTag::UpdateStatusOnline),
            )
            .await?;
        assert!(matches!(
            res2,
            DispatchOutcome::Sent {
                scheduled_offline: false,
                ..
            }
        ));

        // Account 2 in normal mode dispatches SendMessage via the same shared transport and buffer pool.
        let id3 = mux.next_msg_id();
        let res3 = mux
            .dispatch_envelope(
                acc2,
                make_envelope(SessionId(22), id3, TlMethodTag::SendMessage),
            )
            .await?;
        assert!(matches!(res3, DispatchOutcome::Sent { .. }));
        assert_eq!(transport.recorded_count(), 2);
        assert_eq!(mux.buffer_pool().pooled_count(), 1);
        assert!(mux.is_connected(acc1));

        transport.set_fail(true);
        let err = mux
            .dispatch_envelope(acc2, make_envelope(SessionId(22), mux.next_msg_id(), TlMethodTag::Ping))
            .await;
        assert!(matches!(err, Err(MtprotoError::Disconnected { .. })));

        transport.set_fail(false);
        let retried = mux
            .dispatch_with_retry(
                acc2,
                make_envelope(SessionId(22), mux.next_msg_id(), TlMethodTag::Ping),
                2,
            )
            .await?;
        assert!(matches!(retried, DispatchOutcome::Sent { .. }));

        Ok(())
    }

    #[test]
    fn dc_key_binding_and_destroy_queue() -> Result<(), MtprotoError> {
        let transport = Arc::new(MockAsyncTransport::new());
        let mux = MtprotoMultiplexer::new(transport, 4);
        let acc = AccountId(9);
        mux.register_session(acc, 2, SessionId(99), GhostModeConfig::default())?;

        let key_dc2 = AuthKey::from_slice(&[0x33u8; 256])?;
        let key_dc4 = AuthKey::from_slice(&[0x44u8; 256])?;
        mux.bind_dc_key(acc, 2, key_dc2.clone())?;
        mux.bind_dc_key(acc, 4, key_dc4.clone())?;
        mux.set_main_dc(acc, 4)?;

        let queued = mux.schedule_keys_destroy(acc, vec![(2, key_dc2), (4, key_dc4)])?;
        assert_eq!(queued, 2);
        let drained = mux.drain_keys_to_destroy(acc)?;
        assert_eq!(drained.len(), 2);
        assert!(mux.drain_keys_to_destroy(acc)?.is_empty());
        Ok(())
    }

    #[test]
    fn updates_synchronizer_tracks_pts_gaps_and_channel_pts() -> Result<(), MtprotoError> {
        let sync = UpdatesSynchronizer::new();
        let acc = AccountId(3);
        assert_eq!(sync.apply_pts(acc, 0, 0, 1)?, PtsCheckResult::Uninited);

        sync.init_state(
            acc,
            SyncStateSnapshot {
                pts: 100,
                qts: 10,
                date: 1_700_000_000,
                seq: 5,
            },
        )?;
        assert_eq!(
            sync.apply_pts(acc, 0, 101, 1)?,
            PtsCheckResult::Applied { new_pts: 101 }
        );
        assert_eq!(
            sync.apply_pts(acc, 0, 101, 1)?,
            PtsCheckResult::DuplicateIgnored { current_pts: 101 }
        );
        assert_eq!(
            sync.apply_pts(acc, 0, 105, 1)?,
            PtsCheckResult::GapDetected {
                expected_pts: 102,
                received_pts: 105
            }
        );
        assert_eq!(sync.pending_gaps(acc)?, 1);

        let channel_id = 555_001u64;
        assert_eq!(
            sync.apply_pts(acc, channel_id, 50, 1)?,
            PtsCheckResult::Applied { new_pts: 50 }
        );
        assert_eq!(
            sync.apply_pts(acc, channel_id, 52, 2)?,
            PtsCheckResult::Applied { new_pts: 52 }
        );
        assert_eq!(sync.channel_pts(acc, channel_id)?, 52);
        Ok(())
    }

    #[test]
    fn updates_synchronizer_confirms_outgoing_random_id() -> Result<(), MtprotoError> {
        let sync = UpdatesSynchronizer::new();
        let acc = AccountId(7);
        sync.queue_outgoing(acc, 999_111, DialogId(42), MsgId(-1), 1_700_000_123)?;
        let confirmed = sync.confirm_outgoing(acc, 999_111)?;
        assert_eq!(
            confirmed,
            Some(PendingOutgoingMessage {
                random_id: 999_111,
                dialog_id: DialogId(42),
                local_msg_id: MsgId(-1),
                queued_at_unix: 1_700_000_123,
            })
        );
        assert!(sync.confirm_outgoing(acc, 999_111)?.is_none());
        Ok(())
    }

    #[test]
    fn file_transfer_manager_prioritizes_downloads_and_computes_upload_parts()
        -> Result<(), MtprotoError>
    {
        assert_eq!(compute_upload_part_size(512 * 1024), UPLOAD_PART_SIZE_TINY);
        assert_eq!(
            compute_upload_part_size(10 * 1024 * 1024),
            UPLOAD_PART_SIZE_SMALL
        );
        assert_eq!(
            compute_upload_part_size(100 * 1024 * 1024),
            UPLOAD_PART_SIZE_MEDIUM
        );
        assert_eq!(
            compute_upload_part_size(600 * 1024 * 1024),
            UPLOAD_PART_SIZE_LARGE
        );
        assert_eq!(
            compute_upload_part_size(1_200 * 1024 * 1024),
            UPLOAD_PART_SIZE_XLARGE
        );

        let manager = FileTransferManager::new(256 * 1024);
        let acc = AccountId(1);
        let low_id = manager.enqueue_download(acc, 2, 11, 512 * 1024, 1)?;
        let high_id = manager.enqueue_download(acc, 2, 22, 512 * 1024, 10)?;
        let big_upload_id = manager.enqueue_upload(acc, 2, 33, 50 * 1024 * 1024)?;

        let next = manager.next_ready_download(2)?;
        assert_eq!(next.map(|t| t.task_id), Some(high_id));

        let p1 = manager.advance_transfer(high_id, DOWNLOAD_PART_SIZE)?;
        assert_eq!(p1.state, TransferState::Active);
        assert_eq!(p1.part_size, DOWNLOAD_PART_SIZE);
        assert_eq!(manager.active_inflight_bytes(), DOWNLOAD_PART_SIZE);

        let p2 = manager.advance_transfer(low_id, DOWNLOAD_PART_SIZE)?;
        assert_eq!(p2.state, TransferState::Active);
        assert_eq!(manager.active_inflight_bytes(), 2 * DOWNLOAD_PART_SIZE);

        // 3rd concurrent transfer would exceed the 256 KiB cap until a slot frees.
        let blocked = manager.advance_transfer(big_upload_id, DOWNLOAD_PART_SIZE);
        assert!(matches!(blocked, Err(MtprotoError::Framing(_))));

        assert!(manager.cancel_transfer(low_id)?);
        assert_eq!(manager.active_inflight_bytes(), DOWNLOAD_PART_SIZE);

        for _ in 0..3 {
            let _ = manager.advance_transfer(high_id, DOWNLOAD_PART_SIZE)?;
        }
        let finished = manager.progress(high_id)?;
        assert_eq!(finished.map(|t| t.state), Some(TransferState::Completed));
        assert_eq!(manager.active_inflight_bytes(), 0);

        let up_info = manager.progress(big_upload_id)?;
        assert!(up_info.is_some_and(|t| t.is_big_file));
        Ok(())
    }
}
