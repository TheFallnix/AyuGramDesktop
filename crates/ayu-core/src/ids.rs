use bytemuck::{Pod, Zeroable};

/// Peer category shift matching Telegram Desktop's `ChatIdType<Shift>` encoding.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PeerKind {
    User = 0,
    Chat = 1,
    Channel = 2,
    FakeChat = 0x7F,
    Unknown = 0xFF,
}

impl PeerKind {
    #[must_use]
    pub const fn from_shift(shift: u8) -> Self {
        match shift {
            0 => Self::User,
            1 => Self::Chat,
            2 => Self::Channel,
            0x7F => Self::FakeChat,
            _ => Self::Unknown,
        }
    }
}

/// Telegram Desktop encoded 64-bit peer identifier (`(shift << 48) | bare_id`).
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct PeerId(pub u64);

impl PeerId {
    pub const BARE_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn from_parts(kind: PeerKind, bare: u64) -> Self {
        Self((bare & Self::BARE_MASK) | ((kind as u64) << 48))
    }

    #[must_use]
    pub const fn from_user(user_id: UserId) -> Self {
        Self::from_parts(PeerKind::User, user_id.0)
    }

    #[must_use]
    pub const fn from_chat(chat_id: u64) -> Self {
        Self::from_parts(PeerKind::Chat, chat_id)
    }

    #[must_use]
    pub const fn from_channel(channel_id: u64) -> Self {
        Self::from_parts(PeerKind::Channel, channel_id)
    }

    #[must_use]
    pub const fn kind(self) -> PeerKind {
        PeerKind::from_shift(((self.0 >> 48) & 0xFF) as u8)
    }

    #[must_use]
    pub const fn bare_id(self) -> u64 {
        self.0 & Self::BARE_MASK
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Telegram bare 64-bit user identifier.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct UserId(pub u64);

/// Signed 64-bit dialog identifier matching AyuGram/Telegram DB convention.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct DialogId(pub i64);

/// Signed 32-bit message identifier within a dialog/channel.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct MsgId(pub i32);

/// Unique account slot / user key in `MultiAccountOrchestrator`.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct AccountId(pub u64);

/// Forum topic root message identifier (`0` for non-topic dialogs).
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Pod, Zeroable)]
pub struct TopicId(pub i64);
