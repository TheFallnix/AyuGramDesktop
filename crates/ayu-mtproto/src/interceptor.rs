use ayu_core::GhostModeConfig;
use bytes::Bytes;

use crate::protocol::{EncryptedEnvelope, TlMethodTag};

/// Action decided by [`GhostPacketInterceptor`] prior to socket transmission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterceptDecision {
    Pass,
    DropSuppressed,
    Rewrite(TlMethodTag, Bytes),
    PassAndScheduleOffline,
}

/// Inspects outbound MTProto requests and enforces AyuGram Ghost Mode rules.
#[derive(Copy, Clone, Debug, Default)]
pub struct GhostPacketInterceptor;

impl GhostPacketInterceptor {
    #[must_use]
    pub fn evaluate(
        config: GhostModeConfig,
        envelope: &EncryptedEnvelope,
    ) -> InterceptDecision {
        match envelope.method {
            TlMethodTag::ReadHistory | TlMethodTag::ReadChannelHistory => {
                if config.should_send_read_messages() {
                    InterceptDecision::Pass
                } else {
                    InterceptDecision::DropSuppressed
                }
            }
            TlMethodTag::ReadStories => {
                if config.should_send_read_stories() {
                    InterceptDecision::Pass
                } else {
                    InterceptDecision::DropSuppressed
                }
            }
            TlMethodTag::SetTyping => {
                if config.should_send_upload_progress() {
                    InterceptDecision::Pass
                } else {
                    InterceptDecision::DropSuppressed
                }
            }
            TlMethodTag::UpdateStatusOnline => {
                if !config.should_send_online_packets() {
                    InterceptDecision::Rewrite(
                        TlMethodTag::UpdateStatusOffline,
                        Bytes::from_static(b"offline"),
                    )
                } else if config.should_send_offline_after_online() {
                    InterceptDecision::PassAndScheduleOffline
                } else {
                    InterceptDecision::Pass
                }
            }
            TlMethodTag::SendMessage
            | TlMethodTag::UpdateStatusOffline
            | TlMethodTag::Ping
            | TlMethodTag::Raw(_) => InterceptDecision::Pass,
        }
    }
}
