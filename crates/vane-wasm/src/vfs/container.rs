//! 容器格式读写（M2-02 §3 容器格式）。
//!
//! 物理布局（单 OPFS 容器 `vane.db`）：
//! ```text
//! ┌────────────────────────────────────────────┐
//! │ superblock (4 KB)                          │
//! │  magic(4) | format_version(4 LE)           │
//! │  active_meta_slot: u8                      │
//! │  reserved(7)                               │
//! │  meta_offset[2]: u64 LE                    │
//! │  meta_size[2]: u64 LE                      │
//! │  container_size: u64 LE (hint)             │
//! │  reserved ...                              │
//! ├────────────────────────────────────────────┤
//! │ meta_slot_0 (256 KB reserved)              │
//! │  generation:u64 | data_len:u32 | crc32:u32 │
//! │  payload[data_len]                         │
//! │  padding (zeros, 不 CRC)                    │
//! ├────────────────────────────────────────────┤
//! │ meta_slot_1 (双槽，等大预留)                 │
//! ├────────────────────────────────────────────┤
//! │ data area (文件区间，按分配序)               │
//! └────────────────────────────────────────────┘
//! ```
//!
//! Meta slot payload:
//! ```text
//! container_size: u64 LE
//! file_table_count: u32 LE
//! file_table[]: { path_len:u16 LE | path:utf8 | base:u64 LE | size:u64 LE }
//! free_list_count: u32 LE
//! free_list[]: { base:u64 LE | size:u64 LE }
//! ```

use vane_core::types::{Result, VaneError};

// ── 常量 ───────────────────────────────────────────────────────────────────

/// 容器 magic：`VANE`。
pub const MAGIC: &[u8; 4] = b"VANE";
/// 容器格式版本。
pub const FORMAT_VERSION: u32 = 1;
/// superblock 大小（字节）。
pub const SUPERBLOCK_SIZE: u64 = 4096;
/// 单个 meta slot 预留大小（字节）。
pub const META_SLOT_SIZE: u64 = 262_144; // 256 KB
/// meta_slot_0 在容器中的偏移。
pub const META_SLOT_0_OFFSET: u64 = SUPERBLOCK_SIZE;
/// meta_slot_1 在容器中的偏移。
pub const META_SLOT_1_OFFSET: u64 = SUPERBLOCK_SIZE + META_SLOT_SIZE;
/// 数据区起始偏移（superblock + 双 meta slot 之后）。
pub const DATA_OFFSET: u64 = SUPERBLOCK_SIZE + 2 * META_SLOT_SIZE;

/// meta slot 头部大小：generation(8) + data_len(4) + crc32(4) = 16 字节。
const META_HEADER_SIZE: usize = 16;

// ── Extent ─────────────────────────────────────────────────────────────────

/// 文件区间：容器内 `[base, base+size)` 字节。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub base: u64,
    pub size: u64,
}

// ── CRC32 (IEEE 802.3, polynomial 0xEDB88320) ──────────────────────────────

/// 手写 CRC32（避免新依赖）。与 zlib / crc32fast 结果一致。
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

// ── Superblock ─────────────────────────────────────────────────────────────

/// superblock 解析结果。`active_meta_slot` 仅为 hint——recover 始终校验双槽
/// CRC 并取 generation 最大者，不依赖此字段。
#[derive(Clone, Debug)]
pub struct Superblock {
    pub active_meta_slot: u8,
    /// container_size hint（权威值在 meta slot payload）。
    pub container_size: u64,
}

impl Superblock {
    /// 编码到 4 KB buffer。
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![0u8; SUPERBLOCK_SIZE as usize];
        buf[0..4].copy_from_slice(MAGIC);
        buf[4..8].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        buf[8] = self.active_meta_slot;
        // buf[9..16] reserved (zeros)
        buf[16..24].copy_from_slice(&META_SLOT_0_OFFSET.to_le_bytes());
        buf[24..32].copy_from_slice(&META_SLOT_1_OFFSET.to_le_bytes());
        buf[32..40].copy_from_slice(&META_SLOT_SIZE.to_le_bytes());
        buf[40..48].copy_from_slice(&META_SLOT_SIZE.to_le_bytes());
        buf[48..56].copy_from_slice(&self.container_size.to_le_bytes());
        // buf[56..4096] reserved (zeros)
        buf
    }

    /// 从 buffer 解析。仅做 magic 校验，不做 CRC（CRC 在 meta slot 层）。
    pub fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < 56 {
            return Err(VaneError::Io("superblock too short".into()));
        }
        if &buf[0..4] != MAGIC {
            return Err(VaneError::Io("bad magic".into()));
        }
        let active_meta_slot = buf[8];
        let container_size = u64::from_le_bytes(buf[48..56].try_into().unwrap());
        Ok(Self {
            active_meta_slot,
            container_size,
        })
    }
}

// ── MetaSlot ───────────────────────────────────────────────────────────────

/// meta slot 解析结果（自包含：file_table + free_list + container_size）。
#[derive(Clone, Debug)]
pub struct MetaSlot {
    pub generation: u64,
    pub container_size: u64,
    pub file_table: Vec<(String, Extent)>,
    pub free_list: Vec<Extent>,
}

impl MetaSlot {
    /// 编码：header(16) + payload。返回不含 padding 的紧凑字节。
    pub fn encode(&self) -> Result<Vec<u8>> {
        // 编码 payload
        let mut payload = Vec::new();
        payload.extend_from_slice(&self.container_size.to_le_bytes());
        payload.extend_from_slice(&(self.file_table.len() as u32).to_le_bytes());
        for (path, ext) in &self.file_table {
            let pb = path.as_bytes();
            if pb.len() > u16::MAX as usize {
                return Err(VaneError::Io(format!("path too long: {}", pb.len()).into()));
            }
            payload.extend_from_slice(&(pb.len() as u16).to_le_bytes());
            payload.extend_from_slice(pb);
            payload.extend_from_slice(&ext.base.to_le_bytes());
            payload.extend_from_slice(&ext.size.to_le_bytes());
        }
        payload.extend_from_slice(&(self.free_list.len() as u32).to_le_bytes());
        for ext in &self.free_list {
            payload.extend_from_slice(&ext.base.to_le_bytes());
            payload.extend_from_slice(&ext.size.to_le_bytes());
        }

        if payload.len() + META_HEADER_SIZE > META_SLOT_SIZE as usize {
            return Err(VaneError::Io(
                format!(
                    "meta slot overflow: {} + {} > {}",
                    payload.len(),
                    META_HEADER_SIZE,
                    META_SLOT_SIZE
                )
                .into(),
            ));
        }

        let crc = crc32(&payload);
        let mut encoded = Vec::with_capacity(META_HEADER_SIZE + payload.len());
        encoded.extend_from_slice(&self.generation.to_le_bytes());
        encoded.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&crc.to_le_bytes());
        encoded.extend_from_slice(&payload);
        Ok(encoded)
    }

    /// 从 header+payload 字节解码。调用方负责读取足够字节。
    /// 返回 `Ok(None)` 表示无有效 meta slot（空槽 / CRC 失败 / 数据不完整）。
    pub fn decode_from(buf: &[u8]) -> Result<Option<Self>> {
        if buf.len() < META_HEADER_SIZE {
            return Ok(None);
        }
        let generation = u64::from_le_bytes(buf[0..8].try_into().unwrap());
        let data_len = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(buf[12..16].try_into().unwrap());

        if data_len == 0 || data_len + META_HEADER_SIZE > META_SLOT_SIZE as usize {
            return Ok(None);
        }
        let payload = &buf[META_HEADER_SIZE..META_HEADER_SIZE + data_len];
        if payload.len() < data_len {
            return Ok(None); // 数据不完整
        }
        if crc32(payload) != crc {
            return Ok(None); // CRC 校验失败
        }
        Self::decode_payload(generation, payload).map(Some)
    }

    fn decode_payload(generation: u64, payload: &[u8]) -> Result<Self> {
        if payload.len() < 12 {
            return Err(VaneError::Io("meta payload too short".into()));
        }
        let container_size = u64::from_le_bytes(payload[0..8].try_into().unwrap());
        let mut cur = 8;
        let ft_count = u32::from_le_bytes(payload[cur..cur + 4].try_into().unwrap()) as usize;
        cur += 4;

        let mut file_table = Vec::with_capacity(ft_count);
        for _ in 0..ft_count {
            if cur + 2 > payload.len() {
                return Err(VaneError::Io("meta payload truncated in file_table".into()));
            }
            let pl = u16::from_le_bytes(payload[cur..cur + 2].try_into().unwrap()) as usize;
            cur += 2;
            if cur + pl + 16 > payload.len() {
                return Err(VaneError::Io(
                    "meta payload truncated in path/extent".into(),
                ));
            }
            let path = String::from_utf8(payload[cur..cur + pl].to_vec())
                .map_err(|e| VaneError::Io(format!("invalid utf8 path: {e}").into()))?;
            cur += pl;
            let base = u64::from_le_bytes(payload[cur..cur + 8].try_into().unwrap());
            cur += 8;
            let size = u64::from_le_bytes(payload[cur..cur + 8].try_into().unwrap());
            cur += 8;
            file_table.push((path, Extent { base, size }));
        }

        if cur + 4 > payload.len() {
            return Err(VaneError::Io(
                "meta payload truncated in free_list_count".into(),
            ));
        }
        let fl_count = u32::from_le_bytes(payload[cur..cur + 4].try_into().unwrap()) as usize;
        cur += 4;

        let mut free_list = Vec::with_capacity(fl_count);
        for _ in 0..fl_count {
            if cur + 16 > payload.len() {
                return Err(VaneError::Io("meta payload truncated in free_list".into()));
            }
            let base = u64::from_le_bytes(payload[cur..cur + 8].try_into().unwrap());
            cur += 8;
            let size = u64::from_le_bytes(payload[cur..cur + 8].try_into().unwrap());
            cur += 8;
            free_list.push(Extent { base, size });
        }

        Ok(Self {
            generation,
            container_size,
            file_table,
            free_list,
        })
    }
}

/// 返回 meta slot 在容器中的偏移。
pub fn meta_slot_offset(slot: u8) -> u64 {
    if slot == 0 {
        META_SLOT_0_OFFSET
    } else {
        META_SLOT_1_OFFSET
    }
}

// ── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_values() {
        // 标准 CRC32 测试向量（与 zlib/png 一致）
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xe8b7be43);
        assert_eq!(crc32(b"hello"), 0x3610a686);
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }

    #[test]
    fn superblock_round_trip() {
        let sb = Superblock {
            active_meta_slot: 1,
            container_size: DATA_OFFSET + 1024,
        };
        let encoded = sb.encode();
        assert_eq!(encoded.len(), SUPERBLOCK_SIZE as usize);
        let decoded = Superblock::decode(&encoded).unwrap();
        assert_eq!(decoded.active_meta_slot, 1);
        assert_eq!(decoded.container_size, DATA_OFFSET + 1024);
    }

    #[test]
    fn superblock_bad_magic() {
        let mut buf = vec![0u8; SUPERBLOCK_SIZE as usize];
        buf[0..4].copy_from_slice(b"XXXX");
        assert!(Superblock::decode(&buf).is_err());
    }

    #[test]
    fn meta_slot_round_trip() {
        let meta = MetaSlot {
            generation: 42,
            container_size: DATA_OFFSET + 500,
            file_table: vec![
                (
                    "manifest.json".to_string(),
                    Extent {
                        base: DATA_OFFSET,
                        size: 100,
                    },
                ),
                (
                    "wal.log".to_string(),
                    Extent {
                        base: DATA_OFFSET + 100,
                        size: 400,
                    },
                ),
            ],
            free_list: vec![Extent {
                base: DATA_OFFSET + 200,
                size: 50,
            }],
        };
        let encoded = meta.encode().unwrap();
        let decoded = MetaSlot::decode_from(&encoded).unwrap().unwrap();
        assert_eq!(decoded.generation, 42);
        assert_eq!(decoded.container_size, DATA_OFFSET + 500);
        assert_eq!(decoded.file_table.len(), 2);
        assert_eq!(decoded.file_table[0].0, "manifest.json");
        assert_eq!(
            decoded.file_table[0].1,
            Extent {
                base: DATA_OFFSET,
                size: 100
            }
        );
        assert_eq!(decoded.file_table[1].0, "wal.log");
        assert_eq!(decoded.free_list.len(), 1);
        assert_eq!(
            decoded.free_list[0],
            Extent {
                base: DATA_OFFSET + 200,
                size: 50
            }
        );
    }

    #[test]
    fn meta_slot_empty_round_trip() {
        let meta = MetaSlot {
            generation: 0,
            container_size: DATA_OFFSET,
            file_table: vec![],
            free_list: vec![],
        };
        let encoded = meta.encode().unwrap();
        let decoded = MetaSlot::decode_from(&encoded).unwrap().unwrap();
        assert_eq!(decoded.generation, 0);
        assert!(decoded.file_table.is_empty());
        assert!(decoded.free_list.is_empty());
    }

    #[test]
    fn meta_slot_crc_corrupt_returns_none() {
        let meta = MetaSlot {
            generation: 1,
            container_size: DATA_OFFSET,
            file_table: vec![(
                "a".to_string(),
                Extent {
                    base: DATA_OFFSET,
                    size: 10,
                },
            )],
            free_list: vec![],
        };
        let mut encoded = meta.encode().unwrap();
        // 翻转 payload 中一个字节（不翻 header）
        encoded[20] ^= 0xFF;
        let decoded = MetaSlot::decode_from(&encoded).unwrap();
        assert!(decoded.is_none(), "corrupted CRC should return None");
    }

    #[test]
    fn meta_slot_empty_buffer_returns_none() {
        let buf = vec![0u8; 8];
        assert!(MetaSlot::decode_from(&buf).unwrap().is_none());
    }

    #[test]
    fn meta_slot_unicode_path() {
        let meta = MetaSlot {
            generation: 1,
            container_size: DATA_OFFSET,
            file_table: vec![("段/数据.bin".to_string(), Extent { base: 100, size: 5 })],
            free_list: vec![],
        };
        let encoded = meta.encode().unwrap();
        let decoded = MetaSlot::decode_from(&encoded).unwrap().unwrap();
        assert_eq!(decoded.file_table[0].0, "段/数据.bin");
    }
}
