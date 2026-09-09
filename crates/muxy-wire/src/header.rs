use muxy_protocol::{ChannelId, SUPPORTED, V1, Version};

use crate::{MessageKind, WireError};

pub const HEADER_LEN: usize = 11;
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub length: u32,
    pub version: u16,
    pub channel: u32,
    pub kind: u8,
}

impl Header {
    pub fn new(
        payload_len: usize,
        channel: ChannelId,
        kind: MessageKind,
    ) -> Result<Self, WireError> {
        if payload_len > MAX_FRAME - HEADER_LEN {
            return Err(WireError::FrameTooLarge);
        }
        let length =
            u32::try_from(payload_len + HEADER_LEN - 4).map_err(|_| WireError::FrameTooLarge)?;
        Ok(Self {
            length,
            version: V1.0,
            channel: channel.0,
            kind: kind as u8,
        })
    }

    pub fn from_bytes(bytes: [u8; HEADER_LEN]) -> Result<Self, WireError> {
        let header = Self {
            length: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            version: u16::from_le_bytes([bytes[4], bytes[5]]),
            channel: u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
            kind: bytes[10],
        };
        header.validate()?;
        Ok(header)
    }

    pub fn to_bytes(self) -> [u8; HEADER_LEN] {
        let mut bytes = [0; HEADER_LEN];
        bytes[..4].copy_from_slice(&self.length.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.version.to_le_bytes());
        bytes[6..10].copy_from_slice(&self.channel.to_le_bytes());
        bytes[10] = self.kind;
        bytes
    }

    pub fn payload_len(self) -> Result<usize, WireError> {
        let length = usize::try_from(self.length).map_err(|_| WireError::FrameTooLarge)?;
        if length > MAX_FRAME - 4 {
            return Err(WireError::FrameTooLarge);
        }
        length
            .checked_sub(HEADER_LEN - 4)
            .ok_or_else(|| postcard::Error::DeserializeBadEncoding.into())
    }

    pub(crate) fn validate(self) -> Result<(), WireError> {
        self.payload_len()?;
        if !SUPPORTED.contains(&Version(self.version)) {
            return Err(WireError::UnsupportedVersion(self.version));
        }
        MessageKind::from_u8(self.kind)?;
        Ok(())
    }
}
