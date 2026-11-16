use std::sync::Arc;

use bitflags::bitflags;
use bytemuck::{Pod, Zeroable};
use smallvec::SmallVec;

use crate::ids::{DialogId, MsgId, PeerId, TopicId, UserId};

bitflags! {
    /// Packed 32-bit flags for a message, eliminating per-field booleans.
    #[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
    pub struct MessageFlags: u32 {
        const OUTGOING         = 1 << 0;
        const UNREAD           = 1 << 1;
        const MENTIONED        = 1 << 2;
        const SILENT           = 1 << 3;
        const POST             = 1 << 4;
        const FROM_SCHEDULED   = 1 << 5;
        const LEGACY           = 1 << 6;
        const EDIT_HIDE        = 1 << 7;
        const PINNED           = 1 << 8;
        const NO_FORWARDS      = 1 << 9;
        const HAS_VIEWS        = 1 << 10;
        const HAS_REPLIES      = 1 << 11;
        const SERVICE          = 1 << 12;
        const AYU_DELETED      = 1 << 13;
        const AYU_EDITED       = 1 << 14;
        const FILTERED_BY_AYU  = 1 << 15;
    }
}

bitflags! {
    /// Packed 16-bit flags for a dialog row.
    #[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
    pub struct DialogFlags: u16 {
        const PINNED          = 1 << 0;
        const MUTED           = 1 << 1;
        const ARCHIVED        = 1 << 2;
        const UNREAD_MARK     = 1 << 3;
        const IS_FORUM        = 1 << 4;
        const IS_CHANNEL      = 1 << 5;
        const HAS_DRAFT       = 1 << 6;
        const AYU_DELETED     = 1 << 7;
    }
}

bitflags! {
    /// Packed 16-bit Ghost Mode policy flags mirroring `GhostModeAccountSettings`.
    #[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
    pub struct GhostModeFlags: u16 {
        const GHOST_MODE_ACTIVE                = 1 << 0;
        const SEND_READ_MESSAGES               = 1 << 1;
        const SEND_READ_STORIES                = 1 << 2;
        const SEND_ONLINE_PACKETS              = 1 << 3;
        const SEND_UPLOAD_PROGRESS             = 1 << 4;
        const SEND_OFFLINE_AFTER_ONLINE        = 1 << 5;
        const MARK_READ_AFTER_ACTION           = 1 << 6;
        const USE_SCHEDULED_MESSAGES           = 1 << 7;
        const SUGGEST_BEFORE_VIEWING_STORY     = 1 << 8;
        const SEND_READ_MESSAGES_LOCKED        = 1 << 9;
        const SEND_READ_STORIES_LOCKED         = 1 << 10;
        const SEND_ONLINE_PACKETS_LOCKED       = 1 << 11;
        const SEND_UPLOAD_PROGRESS_LOCKED      = 1 << 12;
        const SEND_OFFLINE_AFTER_ONLINE_LOCKED = 1 << 13;
    }
}

impl Default for GhostModeFlags {
    fn default() -> Self {
        Self::SEND_READ_MESSAGES
            | Self::SEND_READ_STORIES
            | Self::SEND_ONLINE_PACKETS
            | Self::SEND_UPLOAD_PROGRESS
            | Self::MARK_READ_AFTER_ACTION
            | Self::SUGGEST_BEFORE_VIEWING_STORY
    }
}

/// Policy for sending silent messages.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum SendWithoutSoundOption {
    #[default]
    Never = 0,
    InGhostMode = 1,
    Always = 2,
}

impl SendWithoutSoundOption {
    #[must_use]
    pub const fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::InGhostMode,
            2 => Self::Always,
            _ => Self::Never,
        }
    }
}

/// Encapsulated per-account Ghost Mode configuration.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct GhostModeConfig {
    flags: GhostModeFlags,
    send_without_sound: SendWithoutSoundOption,
}

impl GhostModeConfig {
    #[must_use]
    pub const fn new(flags: GhostModeFlags, send_without_sound: SendWithoutSoundOption) -> Self {
        Self {
            flags,
            send_without_sound,
        }
    }

    #[must_use]
    pub const fn flags(self) -> GhostModeFlags {
        self.flags
    }

    #[must_use]
    pub const fn send_without_sound(self) -> SendWithoutSoundOption {
        self.send_without_sound
    }

    #[must_use]
    pub const fn is_ghost_mode_active(self) -> bool {
        self.flags.contains(GhostModeFlags::GHOST_MODE_ACTIVE)
    }

    #[must_use]
    pub const fn should_send_read_messages(self) -> bool {
        if self.is_ghost_mode_active() {
            false
        } else {
            self.flags.contains(GhostModeFlags::SEND_READ_MESSAGES)
        }
    }

    #[must_use]
    pub const fn should_send_read_stories(self) -> bool {
        if self.is_ghost_mode_active() {
            false
        } else {
            self.flags.contains(GhostModeFlags::SEND_READ_STORIES)
        }
    }

    #[must_use]
    pub const fn should_send_online_packets(self) -> bool {
        if self.is_ghost_mode_active() {
            false
        } else {
            self.flags.contains(GhostModeFlags::SEND_ONLINE_PACKETS)
        }
    }

    #[must_use]
    pub const fn should_send_upload_progress(self) -> bool {
        if self.is_ghost_mode_active() {
            false
        } else {
            self.flags.contains(GhostModeFlags::SEND_UPLOAD_PROGRESS)
        }
    }

    #[must_use]
    pub const fn should_send_offline_after_online(self) -> bool {
        self.flags.contains(GhostModeFlags::SEND_OFFLINE_AFTER_ONLINE)
    }

    #[must_use]
    pub const fn should_send_without_sound(self) -> bool {
        match self.send_without_sound {
            SendWithoutSoundOption::Never => false,
            SendWithoutSoundOption::InGhostMode => self.is_ghost_mode_active(),
            SendWithoutSoundOption::Always => true,
        }
    }
}

/// Compact media type discriminant.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum MediaKind {
    #[default]
    None = 0,
    Photo = 1,
    Video = 2,
    Document = 3,
    Voice = 4,
    Sticker = 5,
    Audio = 6,
    Animation = 7,
    WebPage = 8,
}

impl MediaKind {
    #[must_use]
    pub const fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Photo,
            2 => Self::Video,
            3 => Self::Document,
            4 => Self::Voice,
            5 => Self::Sticker,
            6 => Self::Audio,
            7 => Self::Animation,
            8 => Self::WebPage,
            _ => Self::None,
        }
    }
}

/// Media cache bucket kind for prioritized per-account RAM eviction.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum MediaCacheKind {
    #[default]
    Thumbnail = 0,
    Avatar = 1,
    Sticker = 2,
    DecodedPhoto = 3,
    DocumentPreview = 4,
}

impl MediaCacheKind {
    #[must_use]
    pub const fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Avatar,
            2 => Self::Sticker,
            3 => Self::DecodedPhoto,
            4 => Self::DocumentPreview,
            _ => Self::Thumbnail,
        }
    }

    #[must_use]
    pub const fn is_heavy(self) -> bool {
        matches!(
            self,
            Self::Sticker | Self::DecodedPhoto | Self::DocumentPreview
        )
    }
}

/// Compact formatting entity discriminant.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum EntityKind {
    #[default]
    Unknown = 0,
    Bold = 1,
    Italic = 2,
    Underline = 3,
    Strikethrough = 4,
    Code = 5,
    Pre = 6,
    TextUrl = 7,
    Mention = 8,
    Spoiler = 9,
    CustomEmoji = 10,
    Blockquote = 11,
}

/// Zero-padding 16-byte message formatting entity span (`Pod`).
///
/// Fields are ordered from largest alignment to smallest to guarantee zero implicit padding.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Pod, Zeroable)]
pub struct MessageEntitySpan {
    pub extra: u64,
    pub offset: u32,
    pub length: u16,
    pub kind: u8,
    pub flags: u8,
}

/// Heap-allocated only when a message actually attaches media (`Option<Box<MediaDescriptor>>`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MediaDescriptor {
    pub file_id: u64,
    pub access_hash: u64,
    pub size_bytes: u64,
    pub mime_type: Box<str>,
    pub local_path: Option<Box<str>>,
    pub thumb_path: Option<Box<str>>,
    pub width: u32,
    pub height: u32,
    pub duration_secs: u32,
    pub kind: MediaKind,
    pub has_spoiler: bool,
}

/// Memory-compact message model kept inside the active viewport window.
///
/// Fields are ordered strictly from largest to smallest alignment (`8 -> 4 -> 2 -> 1`)
/// to avoid struct padding waste, using `Box<str>`, `Option<Arc<str>>`, `SmallVec`,
/// and niche-optimized `Option<Box<MediaDescriptor>>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactMessage {
    pub dialog_id: DialogId,
    pub sender_id: PeerId,
    pub grouped_id: u64,
    pub text: Box<str>,
    pub author_name: Option<Arc<str>>,
    pub media: Option<Box<MediaDescriptor>>,
    pub entities: SmallVec<[MessageEntitySpan; 2]>,
    pub id: MsgId,
    pub reply_to_msg_id: MsgId,
    pub date_unix: i32,
    pub edit_date_unix: i32,
    pub views: u32,
    pub flags: MessageFlags,
}

impl CompactMessage {
    /// Approximate heap + inline footprint in bytes for memory budgeting.
    #[must_use]
    pub fn estimated_memory_bytes(&self) -> usize {
        let mut total = std::mem::size_of::<Self>() + self.text.len();
        if let Some(ref author) = self.author_name {
            total += author.len();
        }
        if let Some(ref media) = self.media {
            total += std::mem::size_of::<MediaDescriptor>() + media.mime_type.len();
            if let Some(ref path) = media.local_path {
                total += path.len();
            }
            if let Some(ref thumb) = media.thumb_path {
                total += thumb.len();
            }
        }
        if self.entities.spilled() {
            total += self.entities.capacity() * std::mem::size_of::<MessageEntitySpan>();
        }
        total
    }
}

/// Memory-compact dialog list row ordered largest-to-smallest alignment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactDialog {
    pub dialog_id: DialogId,
    pub peer_id: PeerId,
    pub title: Box<str>,
    pub draft_text: Option<Box<str>>,
    pub top_message_id: MsgId,
    pub read_inbox_max_id: MsgId,
    pub read_outbox_max_id: MsgId,
    pub last_message_date: i32,
    pub unread_count: u32,
    pub unread_mentions_count: u32,
    pub folder_id: i32,
    pub flags: DialogFlags,
}

impl CompactDialog {
    /// Approximate heap + inline footprint in bytes for memory budgeting.
    #[must_use]
    pub fn estimated_memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title.len()
            + self.draft_text.as_ref().map_or(0, |d| d.len())
    }
}

/// AyuGram preserved deleted message record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AyuDeletedMessage {
    pub user_id: UserId,
    pub topic_id: TopicId,
    pub entity_create_date: i32,
    pub message: CompactMessage,
}

/// AyuGram preserved message edit revision record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AyuEditedRevision {
    pub user_id: UserId,
    pub entity_create_date: i32,
    pub message: CompactMessage,
}

/// AyuGram regex message filter rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegexFilterRule {
    pub id: Box<[u8]>,
    pub pattern: Box<str>,
    pub dialog_id: Option<DialogId>,
    pub enabled: bool,
    pub reversed: bool,
    pub case_insensitive: bool,
}

/// Excludes a global (shared) regex filter rule from a specific dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexFilterGlobalExclusion {
    pub dialog_id: DialogId,
    pub filter_id: Box<[u8]>,
}
