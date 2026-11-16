use smallvec::SmallVec;

use crate::error::CoreError;
use crate::ids::UserId;

/// Authorization state machine for a single Telegram account.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum AuthState {
    #[default]
    Unauthorized = 0,
    WaitingForPhone = 1,
    WaitingForCode = 2,
    WaitingForPassword2FA = 3,
    Authorized = 4,
    LoggingOut = 5,
}

impl AuthState {
    #[must_use]
    pub const fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::WaitingForPhone,
            2 => Self::WaitingForCode,
            3 => Self::WaitingForPassword2FA,
            4 => Self::Authorized,
            5 => Self::LoggingOut,
            _ => Self::Unauthorized,
        }
    }
}

/// 256-byte MTProto DC authorization key bound to a specific data center.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DcAuthKeyEntry {
    pub key_bytes: Box<[u8; 256]>,
    pub dc_id: i32,
    pub is_temporary: bool,
    pub to_destroy: bool,
}

impl DcAuthKeyEntry {
    #[must_use]
    pub fn new(dc_id: i32, key_bytes: [u8; 256], is_temporary: bool, to_destroy: bool) -> Self {
        Self {
            key_bytes: Box::new(key_bytes),
            dc_id,
            is_temporary,
            to_destroy,
        }
    }
}

/// Binary-compatible representation of `Main::Account::serializeMtpAuthorization`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MtpAuthorizationBlob {
    pub user_id: UserId,
    pub main_dc_id: i32,
    pub keys: SmallVec<[DcAuthKeyEntry; 4]>,
    pub keys_to_destroy: SmallVec<[DcAuthKeyEntry; 2]>,
}

impl MtpAuthorizationBlob {
    /// Sentinel tag matching `kWideIdsTag` (`~uint64(0)`) in `main_account.cpp`.
    pub const WIDE_IDS_TAG: u64 = u64::MAX;

    /// Encodes authorization metadata and DC keys in Telegram Desktop's big-endian `QDataStream` layout.
    #[must_use]
    pub fn encode_tdesktop(&self) -> Vec<u8> {
        let key_record_size = 4usize + 256usize;
        let total_size = 8usize
            + 8usize
            + 4usize
            + 4usize
            + self.keys.len().saturating_mul(key_record_size)
            + 4usize
            + self.keys_to_destroy.len().saturating_mul(key_record_size);

        let mut out = Vec::with_capacity(total_size);
        out.extend_from_slice(&Self::WIDE_IDS_TAG.to_be_bytes());
        out.extend_from_slice(&self.user_id.0.to_be_bytes());
        out.extend_from_slice(&self.main_dc_id.to_be_bytes());

        let write_key_list = |buf: &mut Vec<u8>, list: &[DcAuthKeyEntry]| {
            let count = list.len().min(i32::MAX as usize) as i32;
            buf.extend_from_slice(&count.to_be_bytes());
            for entry in list.iter().take(count as usize) {
                buf.extend_from_slice(&entry.dc_id.to_be_bytes());
                buf.extend_from_slice(entry.key_bytes.as_slice());
            }
        };

        write_key_list(&mut out, &self.keys);
        write_key_list(&mut out, &self.keys_to_destroy);
        out
    }

    /// Decodes both modern (`WIDE_IDS_TAG`) and legacy 32-bit `MTPAuthorization` blobs.
    pub fn decode_tdesktop(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut cursor = bytes;
        let legacy_user_id = read_i32_be(&mut cursor)?;
        let legacy_main_dc_id = read_i32_be(&mut cursor)?;
        let combined = ((u64::from(legacy_user_id as u32)) << 32)
            | u64::from(legacy_main_dc_id as u32);

        let (user_id, main_dc_id) = if combined == Self::WIDE_IDS_TAG {
            let uid = read_u64_be(&mut cursor)?;
            let dc = read_i32_be(&mut cursor)?;
            (UserId(uid), dc)
        } else {
            if legacy_user_id < 0 {
                return Err(CoreError::CorruptAuthBlob("negative legacy user id"));
            }
            (UserId(legacy_user_id as u64), legacy_main_dc_id)
        };

        let keys_raw = read_key_list(&mut cursor, false)?;
        let destroy_raw = read_key_list(&mut cursor, true)?;

        Ok(Self {
            user_id,
            main_dc_id,
            keys: keys_raw.into_iter().collect(),
            keys_to_destroy: destroy_raw.into_iter().collect(),
        })
    }
}

fn read_bytes<'a>(cursor: &mut &'a [u8], len: usize) -> Result<&'a [u8], CoreError> {
    if cursor.len() < len {
        return Err(CoreError::CorruptAuthBlob("unexpected end of buffer"));
    }
    let (head, tail) = cursor.split_at(len);
    *cursor = tail;
    Ok(head)
}

fn read_i32_be(cursor: &mut &[u8]) -> Result<i32, CoreError> {
    let slice = read_bytes(cursor, 4)?;
    let arr: [u8; 4] = slice
        .try_into()
        .map_err(|_| CoreError::CorruptAuthBlob("invalid i32 slice"))?;
    Ok(i32::from_be_bytes(arr))
}

fn read_u64_be(cursor: &mut &[u8]) -> Result<u64, CoreError> {
    let slice = read_bytes(cursor, 8)?;
    let arr: [u8; 8] = slice
        .try_into()
        .map_err(|_| CoreError::CorruptAuthBlob("invalid u64 slice"))?;
    Ok(u64::from_be_bytes(arr))
}

fn read_key_list(
    cursor: &mut &[u8],
    to_destroy: bool,
) -> Result<Vec<DcAuthKeyEntry>, CoreError> {
    let count_raw = read_i32_be(cursor)?;
    if count_raw < 0 {
        return Err(CoreError::CorruptAuthBlob("negative key count"));
    }
    let count = count_raw as usize;
    let needed = count
        .checked_mul(4usize + 256usize)
        .ok_or(CoreError::CorruptAuthBlob("key list size overflow"))?;
    if cursor.len() < needed {
        return Err(CoreError::CorruptAuthBlob("truncated key list"));
    }

    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let dc_id = read_i32_be(cursor)?;
        let key_slice = read_bytes(cursor, 256)?;
        let key_arr: [u8; 256] = key_slice
            .try_into()
            .map_err(|_| CoreError::CorruptAuthBlob("invalid 256-byte key"))?;
        out.push(DcAuthKeyEntry::new(dc_id, key_arr, false, to_destroy));
    }
    Ok(out)
}
