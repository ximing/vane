//! vane-fuzz targets 的共享字节→结构 decoder。
//!
//! 设计取舍：不引 `arbitrary` crate（设计 §3.2 Cargo.toml 只列 libfuzzer-sys；
//! arbitrary 虽大概率不触黑名单，但多一个传递依赖多一份 deny 风险）。自研轻量
//! ByteCursor：从 libfuzzer 提供的 `&[u8]` 确定性地消费字节构造结构化输入；
//! 字节耗尽时返回 0（libfuzzer corpus 普遍短，0 字节是合法边界输入）。
//!
//! 不变量：decoder 自身绝不 panic（全用 `get`+`unwrap_or`+`saturating_add`）。

/// 从 fuzzer 字节流确定性消费的游标。
pub struct ByteCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// 消费 1 字节；耗尽时返 0。
    pub fn u8(&mut self) -> u8 {
        let b = *self.data.get(self.pos).unwrap_or(&0);
        self.pos = self.pos.saturating_add(1);
        b
    }

    /// 消费至多 4 字节（LE）为 u32；不足补 0。
    pub fn u32_le(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        for i in 0..4 {
            buf[i] = *self.data.get(self.pos + i).unwrap_or(&0);
        }
        self.pos = self.pos.saturating_add(4);
        u32::from_le_bytes(buf)
    }

    /// 消费 1 长度前缀字节（cap 32）+ len 字节为 String。
    /// lossy UTF-8 转换：畸形 unicode 不 panic（返回替换字符）。
    pub fn small_string(&mut self) -> String {
        let len = (self.u8() as usize).min(32);
        let mut buf = Vec::with_capacity(len);
        for _ in 0..len {
            buf.push(self.u8());
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// 消费 n×4 字节为 Vec<f32>（每 4 字节 LE）。
    /// NaN/Inf 过滤为 0.0——保 score 算术良定义（设计 §3.3 proptest 同考量）。
    pub fn f32_vec(&mut self, n: usize) -> Vec<f32> {
        (0..n)
            .map(|_| {
                let mut buf = [0u8; 4];
                for i in 0..4 {
                    buf[i] = *self.data.get(self.pos + i).unwrap_or(&0);
                }
                self.pos = self.pos.saturating_add(4);
                let v = f32::from_le_bytes(buf);
                if v.is_nan() || v.is_infinite() {
                    0.0
                } else {
                    v
                }
            })
            .collect()
    }

    /// 消费 1 字节 LSB 为 bool。
    pub fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }
}

/// 构造简单 schema：1 个 Vector 字段（给定 dim + Cosine 度量），可选 1 个 Text 字段。
/// dim 经调用方 clamp 到合法区间（1..=16），Schema::new 不会 Err。
pub fn build_schema(with_text: bool, dim: u32) -> vane_core::types::Schema {
    let mut fields: Vec<(String, vane_core::types::FieldDef)> = Vec::new();
    if with_text {
        fields.push(("body".into(), vane_core::types::FieldDef::Text));
    }
    fields.push((
        "v".into(),
        vane_core::types::FieldDef::Vector {
            dim,
            metric: vane_core::types::Metric::Cosine,
        },
    ));
    vane_core::types::Schema::new(fields).expect("schema with 1 vector field is valid")
}
