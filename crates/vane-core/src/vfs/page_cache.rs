use crate::types::Result;
use crate::vfs::Vfs;
use std::collections::HashMap;

/// SPEC §6.1 LRU 页缓存。read-through：未命中则从 Vfs 加载整页。
///
/// 签名遵循 M0 Global Interface Contracts（§01-vfs 产出）：
/// `read` / `invalidate` 取 `&mut self`，无 interior mutability。
pub struct PageCache {
    inner: Inner,
    page_size: usize,
}

struct Inner {
    pages: HashMap<(String, u64), Vec<u8>>, // (path, page_index) -> data
    order: Vec<(String, u64)>,              // LRU 顺序（尾 = 最近用）
    capacity_bytes: usize,
    used_bytes: usize,
}

impl PageCache {
    pub fn new(capacity_bytes: usize, page_size: usize) -> Self {
        Self {
            inner: Inner {
                pages: HashMap::new(),
                order: Vec::new(),
                capacity_bytes,
                used_bytes: 0,
            },
            page_size,
        }
    }

    pub fn read(&mut self, vfs: &dyn Vfs, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut result = vec![0u8; len];
        let mut remaining = len;
        let mut cur_off = offset;
        let mut out_off = 0usize;

        while remaining > 0 {
            let page_idx = cur_off / self.page_size as u64;
            let page_off = (cur_off % self.page_size as u64) as usize;
            let chunk = remaining.min(self.page_size - page_off);

            let page_data = {
                let key = (path.to_string(), page_idx);
                let hit = self.inner.pages.get(&key).cloned();
                match hit {
                    Some(data) => {
                        // 命中：移动到 LRU 尾
                        self.inner.touch(path.to_string(), page_idx);
                        data
                    }
                    None => {
                        // 未命中：从 vfs 加载整页
                        let mut page_buf = vec![0u8; self.page_size];
                        let page_start = page_idx * self.page_size as u64;
                        let n = vfs.read_at(path, &mut page_buf, page_start)?;
                        page_buf.truncate(n);
                        self.inner.put(path.to_string(), page_idx, page_buf.clone());
                        page_buf
                    }
                }
            };

            let copy_n = chunk.min(page_data.len().saturating_sub(page_off));
            if copy_n > 0 {
                result[out_off..out_off + copy_n]
                    .copy_from_slice(&page_data[page_off..page_off + copy_n]);
            }
            out_off += chunk;
            cur_off += chunk as u64;
            remaining -= chunk;
        }
        Ok(result)
    }

    pub fn invalidate(&mut self, path: &str) {
        let keys_to_remove: Vec<(String, u64)> = self
            .inner
            .pages
            .keys()
            .filter(|(p, _)| p == path)
            .cloned()
            .collect();
        for k in keys_to_remove {
            if let Some(data) = self.inner.pages.remove(&k) {
                self.inner.used_bytes -= data.len();
            }
        }
        self.inner.order.retain(|(p, _)| p != path);
    }
}

impl Inner {
    fn touch(&mut self, path: String, page_idx: u64) {
        let key = (path, page_idx);
        if let Some(pos) = self.order.iter().position(|k| *k == key) {
            self.order.remove(pos);
        }
        self.order.push(key);
    }

    fn put(&mut self, path: String, page_idx: u64, data: Vec<u8>) {
        let key = (path, page_idx);
        let page_len = data.len();
        // 淘汰直到有空间
        while self.used_bytes + page_len > self.capacity_bytes && !self.order.is_empty() {
            let evict = self.order.remove(0);
            if let Some(d) = self.pages.remove(&evict) {
                self.used_bytes -= d.len();
            }
        }
        self.used_bytes += page_len;
        self.pages.insert(key.clone(), data);
        self.order.push(key);
    }
}
