use rustc_hash::{FxHashMap, FxHashSet};
use std::io::{self, Read, Write};
use sha2::{Sha256, Digest};
use memchr::{memchr, memchr2};

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() { return Some(0); }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn raw_line_matches(line: &[u8], key: &[u8], value: &[u8]) -> bool {
    let mut pattern = Vec::with_capacity(key.len() + 3);
    pattern.push(b'"');
    pattern.extend_from_slice(key);
    pattern.push(b'"');
    let Some(pos) = find_subslice(line, &pattern) else { return false; };
    let rest = &line[pos + pattern.len()..];
    let mut ri = 0;
    while ri < rest.len() && (rest[ri] == b' ' || rest[ri] == b':') { ri += 1; }
    let rest = &rest[ri..];
    if rest.starts_with(b"\"") {
        memchr(b'"', &rest[1..]).map_or(false, |end| &rest[1..1 + end] == value)
    } else {
        rest.starts_with(value) &&
            rest.get(value.len()).map_or(true, |&b| !b.is_ascii_alphanumeric() && b != b'.' && b != b'-')
    }
}

pub const MAGIC: &[u8; 4] = b"LUMP";
pub const VERSION_MAJOR: u8 = 0x08;
pub const VERSION_MINOR: u8 = 0x00;
const HEADER_SIZE: usize = 6;

const TYPE_VARINT: u8 = 0;
const TYPE_STRING: u8 = 1;
const TYPE_LITERAL: u8 = 2;
const TYPE_STRING_LIT: u8 = 3;

const CARD_SAMPLES: u32 = 100;
const CARD_THRESH: f32 = 0.5;

fn trim_ascii_slice(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(s.len());
    let end = s.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(start, |p| p + 1);
    &s[start..end]
}

fn is_number_byte(b: u8) -> bool {
    b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+' || b == b'e' || b == b'E'
}

fn encode_zigzag_varint(n: i64, out: &mut Vec<u8>) {
    let mut z = ((n << 1) ^ (n >> 63)) as u64;
    loop {
        if z < 0x80 { out.push(z as u8); return; }
        out.push((z as u8 & 0x7F) | 0x80);
        z >>= 7;
    }
}

fn decode_zigzag_varint(data: &[u8], cursor: &mut usize) -> i64 {
    let mut z: u64 = 0;
    let mut shift = 0u32;
    loop {
        let b = data[*cursor];
        *cursor += 1;
        z |= ((b & 0x7F) as u64) << shift;
        shift += 7;
        if b < 0x80 { break; }
    }
    ((z >> 1) as i64) ^ -((z & 1) as i64)
}

#[inline]
fn push_u32(v: u32, buf: &mut Vec<u8>) {
    buf.extend_from_slice(&v.to_le_bytes());
}

#[derive(PartialEq)]
pub enum InputFormat {
    JsonLines,
    JsonArray,
    Csv,
}

impl InputFormat {
    pub fn label(&self) -> &'static str {
        match self {
            InputFormat::JsonLines => "JSONL",
            InputFormat::JsonArray => "JSON",
            InputFormat::Csv => "CSV",
        }
    }
}

pub struct LumpiEngine {
    schema_dict: FxHashMap<Vec<u8>, u16>,
    next_key_id: u16,
    value_dict: FxHashMap<Vec<u8>, u32>,
    next_val_id: u32,
    dict_bytes: Vec<u8>,
    dict_lengths: Vec<u32>,
    keys_stream: Vec<u16>,
    types_stream: Vec<u8>,
    string_ids_stream: Vec<u32>,
    varint_stream: Vec<u8>,
    literal_stream: Vec<u8>,
    literal_lengths: Vec<u16>,
    fields_per_row: Vec<u16>,
    key_str_seen: FxHashMap<u16, (u32, u32)>,
    high_card_keys: FxHashSet<u16>,
}

#[derive(PartialEq)]
enum FsmState {
    ObjectStart,
    Key,
    Colon,
    ValueString,
    ValueNumber,
}

impl LumpiEngine {
    pub fn new() -> Self {
        LumpiEngine {
            schema_dict: FxHashMap::default(),
            next_key_id: 0,
            value_dict: FxHashMap::default(),
            next_val_id: 0,
            dict_bytes: Vec::new(),
            dict_lengths: Vec::new(),
            keys_stream: Vec::new(),
            types_stream: Vec::new(),
            string_ids_stream: Vec::new(),
            varint_stream: Vec::new(),
            literal_stream: Vec::new(),
            literal_lengths: Vec::new(),
            fields_per_row: Vec::new(),
            key_str_seen: FxHashMap::default(),
            high_card_keys: FxHashSet::default(),
        }
    }

    pub fn clear(&mut self) {
        self.schema_dict.clear();
        self.next_key_id = 0;
        self.value_dict.clear();
        self.next_val_id = 0;
        self.dict_bytes.clear();
        self.dict_lengths.clear();
        self.keys_stream.clear();
        self.types_stream.clear();
        self.string_ids_stream.clear();
        self.varint_stream.clear();
        self.literal_stream.clear();
        self.literal_lengths.clear();
        self.fields_per_row.clear();
        self.key_str_seen.clear();
        self.high_card_keys.clear();
    }

    pub fn was_structured(&self) -> bool {
        !self.keys_stream.is_empty()
    }

    pub fn detect_format(data: &[u8]) -> InputFormat {
        let mut i = 0;
        while i < data.len() && data[i].is_ascii_whitespace() { i += 1; }
        if i >= data.len() { return InputFormat::JsonLines; }
        if data[i] == b'[' { return InputFormat::JsonArray; }
        if data[i] == b'{' { return InputFormat::JsonLines; }
        InputFormat::Csv
    }

    fn strip_json_array(data: &[u8]) -> &[u8] {
        let mut start = 0;
        while start < data.len() && data[start].is_ascii_whitespace() { start += 1; }
        if start < data.len() && data[start] == b'[' { start += 1; }
        let mut end = data.len();
        while end > start && data[end - 1].is_ascii_whitespace() { end -= 1; }
        if end > start && data[end - 1] == b']' { end -= 1; }
        &data[start..end]
    }

    fn intern_key(&mut self, key: &[u8]) -> u16 {
        if let Some(&id) = self.schema_dict.get(key) { return id; }
        let id = self.next_key_id;
        self.next_key_id += 1;
        self.schema_dict.insert(key.to_vec(), id);
        id
    }

    fn intern_string_value(&mut self, val: &[u8]) -> u32 {
        if let Some(&id) = self.value_dict.get(val) { return id; }
        let id = self.next_val_id;
        self.next_val_id += 1;
        self.dict_bytes.extend_from_slice(val);
        self.dict_lengths.push(val.len() as u32);
        self.value_dict.insert(val.to_vec(), id);
        id
    }

    fn emit_string_value(&mut self, key_id: u16, val: &[u8]) {
        if self.high_card_keys.contains(&key_id) {
            self.types_stream.push(TYPE_STRING_LIT);
            self.literal_stream.extend_from_slice(val);
            self.literal_lengths.push(val.len() as u16);
            return;
        }
        let was_new = !self.value_dict.contains_key(val);
        let dict_id = self.intern_string_value(val);
        let entry = self.key_str_seen.entry(key_id).or_insert((0, 0));
        entry.0 += 1;
        if was_new { entry.1 += 1; }
        if entry.0 >= CARD_SAMPLES && entry.1 as f32 / entry.0 as f32 > CARD_THRESH {
            self.high_card_keys.insert(key_id);
        }
        self.types_stream.push(TYPE_STRING);
        self.string_ids_stream.push(dict_id);
    }

    fn emit_number_value(&mut self, bytes: &[u8]) {
        match std::str::from_utf8(bytes).ok().and_then(|s| s.parse::<i64>().ok()) {
            Some(n) => {
                self.types_stream.push(TYPE_VARINT);
                encode_zigzag_varint(n, &mut self.varint_stream);
            }
            None => {
                self.types_stream.push(TYPE_LITERAL);
                self.literal_stream.extend_from_slice(bytes);
                self.literal_lengths.push(bytes.len() as u16);
            }
        }
    }

    fn emit_csv_field(&mut self, field: &[u8], key_id: u16) {
        self.keys_stream.push(key_id);
        let trimmed = trim_ascii_slice(field);
        if trimmed.len() >= 2 && trimmed[0] == b'"' && trimmed[trimmed.len() - 1] == b'"' {
            let inner = &trimmed[1..trimmed.len() - 1];
            self.emit_string_value(key_id, inner);
        } else if !trimmed.is_empty() && trimmed.iter().all(|&b| is_number_byte(b)) {
            self.emit_number_value(trimmed);
        } else {
            self.emit_string_value(key_id, trimmed);
        }
    }

    fn parse_csv(&mut self, data: &[u8]) -> bool {
        let header_end = match memchr(b'\n', data) {
            Some(pos) => pos,
            None => return false,
        };
        let header_line = {
            let h = &data[..header_end];
            if h.last() == Some(&b'\r') { &h[..h.len() - 1] } else { h }
        };
        let mut headers: Vec<&[u8]> = Vec::new();
        let mut col_start = 0;
        let mut search_from = 0;
        loop {
            match memchr(b',', &header_line[search_from..]) {
                Some(rel) => {
                    let i = search_from + rel;
                    let mut col = trim_ascii_slice(&header_line[col_start..i]);
                    if col.len() >= 2 && col[0] == b'"' && col[col.len() - 1] == b'"' {
                        col = &col[1..col.len() - 1];
                    }
                    headers.push(col);
                    col_start = i + 1;
                    search_from = col_start;
                }
                None => {
                    let mut col = trim_ascii_slice(&header_line[col_start..]);
                    if col.len() >= 2 && col[0] == b'"' && col[col.len() - 1] == b'"' {
                        col = &col[1..col.len() - 1];
                    }
                    headers.push(col);
                    break;
                }
            }
        }
        if headers.is_empty() { return false; }
        let key_ids: Vec<u16> = headers.iter().map(|h| self.intern_key(h)).collect();
        let num_cols = key_ids.len();
        let mut cursor = header_end + 1;
        while cursor < data.len() {
            let line_end = match memchr(b'\n', &data[cursor..]) {
                Some(pos) => cursor + pos,
                None => data.len(),
            };
            let line = {
                let l = &data[cursor..line_end];
                if l.last() == Some(&b'\r') { &l[..l.len() - 1] } else { l }
            };
            cursor = line_end + 1;
            if line.is_empty() || line.iter().all(|b| b.is_ascii_whitespace()) { continue; }
            let mut field_idx = 0usize;
            let mut field_start = 0usize;
            let mut search = 0usize;
            loop {
                match memchr(b',', &line[search..]) {
                    Some(rel) => {
                        let i = search + rel;
                        if field_idx >= num_cols { return false; }
                        self.emit_csv_field(&line[field_start..i], key_ids[field_idx]);
                        field_idx += 1;
                        field_start = i + 1;
                        search = field_start;
                    }
                    None => {
                        if field_idx >= num_cols { return false; }
                        self.emit_csv_field(&line[field_start..], key_ids[field_idx]);
                        field_idx += 1;
                        break;
                    }
                }
            }
            if field_idx > 0 { self.fields_per_row.push(field_idx as u16); }
        }
        !self.keys_stream.is_empty()
    }

    pub fn compress_buffer(&mut self, raw_data: &[u8]) -> io::Result<(Vec<u8>, String)> {
        let format = Self::detect_format(raw_data);
        let parse_data = if format == InputFormat::JsonArray {
            Self::strip_json_array(raw_data)
        } else {
            raw_data
        };

        let mut is_structured = true;

        if format == InputFormat::Csv {
            if !self.parse_csv(parse_data) {
                self.clear();
                is_structured = false;
            }
        } else {
            let len = parse_data.len();
            let mut cursor = 0;
            let mut state = FsmState::ObjectStart;
            let mut field_count: u16 = 0;
            let mut val_start = 0usize;
            let mut cur_key_id: u16 = 0;

            'parse: while cursor < len {
                match state {
                    FsmState::ObjectStart => {
                        match memchr(b'{', &parse_data[cursor..]) {
                            Some(pos) => {
                                for &b in &parse_data[cursor..cursor + pos] {
                                    if !b.is_ascii_whitespace() && b != b',' {
                                        is_structured = false;
                                        break 'parse;
                                    }
                                }
                                cursor += pos + 1;
                                field_count = 0;
                                state = FsmState::Key;
                            }
                            None => break,
                        }
                    }
                    FsmState::Key => {
                        match memchr2(b'"', b'}', &parse_data[cursor..]) {
                            Some(pos) => cursor += pos,
                            None => break,
                        }
                        if parse_data[cursor] == b'}' {
                            cursor += 1;
                            if field_count > 0 {
                                self.fields_per_row.push(field_count);
                                field_count = 0;
                            }
                            match memchr(b'{', &parse_data[cursor..]) {
                                Some(pos) => { cursor += pos + 1; state = FsmState::Key; }
                                None => break,
                            }
                            continue;
                        }
                        cursor += 1;
                        let key_len = match memchr(b'"', &parse_data[cursor..]) {
                            Some(pos) => pos,
                            None => { is_structured = false; break 'parse; }
                        };
                        let key_slice = &parse_data[cursor..cursor + key_len];
                        cursor += key_len + 1;
                        let id = self.intern_key(key_slice);
                        cur_key_id = id;
                        self.keys_stream.push(id);
                        state = FsmState::Colon;
                    }
                    FsmState::Colon => {
                        match memchr(b':', &parse_data[cursor..]) {
                            Some(pos) => cursor += pos + 1,
                            None => { is_structured = false; break 'parse; }
                        }
                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor >= len { is_structured = false; break 'parse; }
                        match parse_data[cursor] {
                            b'"' => { cursor += 1; val_start = cursor; state = FsmState::ValueString; }
                            b'{' | b'[' => { is_structured = false; break 'parse; }
                            _ => { val_start = cursor; state = FsmState::ValueNumber; }
                        }
                    }
                    FsmState::ValueString => {
                        loop {
                            match memchr(b'"', &parse_data[cursor..]) {
                                Some(pos) => {
                                    cursor += pos;
                                    let mut escapes = 0usize;
                                    let mut ci = cursor.saturating_sub(1);
                                    while ci >= val_start && parse_data[ci] == b'\\' {
                                        escapes += 1;
                                        if ci == 0 { break; }
                                        ci -= 1;
                                    }
                                    if escapes % 2 == 1 { cursor += 1; } else { break; }
                                }
                                None => { is_structured = false; break 'parse; }
                            }
                        }
                        let val_slice = &parse_data[val_start..cursor];
                        cursor += 1;
                        self.emit_string_value(cur_key_id, val_slice);
                        field_count += 1;
                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor < len && parse_data[cursor] == b',' { cursor += 1; }
                        state = FsmState::Key;
                    }
                    FsmState::ValueNumber => {
                        let (val_slice, new_cursor) = match memchr2(b',', b'}', &parse_data[cursor..]) {
                            Some(pos) => {
                                let end = cursor + pos;
                                let mut te = end;
                                while te > val_start && parse_data[te - 1].is_ascii_whitespace() { te -= 1; }
                                (&parse_data[val_start..te], end)
                            }
                            None => {
                                let mut te = len;
                                while te > val_start && parse_data[te - 1].is_ascii_whitespace() { te -= 1; }
                                (&parse_data[val_start..te], len)
                            }
                        };
                        self.emit_number_value(val_slice);
                        field_count += 1;
                        cursor = new_cursor;
                        if cursor < len && parse_data[cursor] == b',' { cursor += 1; }
                        state = FsmState::Key;
                    }
                }
            }

            if field_count > 0 && state != FsmState::ObjectStart {
                self.fields_per_row.push(field_count);
            }
            if self.keys_stream.is_empty() { is_structured = false; }
        }

        if !is_structured { self.clear(); }

        let (hash_result, uncompressed_payload) = if !is_structured {
            let mut hasher = Sha256::new();
            hasher.update(raw_data);
            let hash = hex::encode(hasher.finalize());
            let mut p = Vec::with_capacity(raw_data.len() + 68);
            p.extend_from_slice(hash.as_bytes());
            p.extend_from_slice(&0xFFFFFFFF_u32.to_le_bytes());
            p.extend_from_slice(raw_data);
            (hash, p)
        } else {
            let mut bp = Vec::with_capacity(
                self.dict_bytes.len() + self.dict_lengths.len() * 4 +
                self.keys_stream.len() * 2 + self.types_stream.len() +
                self.string_ids_stream.len() * 4 + self.varint_stream.len() +
                self.literal_stream.len() + self.literal_lengths.len() +
                self.fields_per_row.len() * 2 + 64
            );
            push_u32(self.dict_bytes.len() as u32, &mut bp);
            bp.extend_from_slice(&self.dict_bytes);
            push_u32(self.dict_lengths.len() as u32, &mut bp);
            for &l in &self.dict_lengths { push_u32(l, &mut bp); }
            push_u32(self.keys_stream.len() as u32, &mut bp);
            for &k in &self.keys_stream { bp.extend_from_slice(&k.to_le_bytes()); }
            push_u32(self.types_stream.len() as u32, &mut bp);
            bp.extend_from_slice(&self.types_stream);
            push_u32(self.string_ids_stream.len() as u32, &mut bp);
            for &v in &self.string_ids_stream { push_u32(v, &mut bp); }
            push_u32(self.varint_stream.len() as u32, &mut bp);
            bp.extend_from_slice(&self.varint_stream);
            push_u32(self.literal_stream.len() as u32, &mut bp);
            bp.extend_from_slice(&self.literal_stream);
            push_u32(self.literal_lengths.len() as u32, &mut bp);
            for &l in &self.literal_lengths { bp.extend_from_slice(&l.to_le_bytes()); }
            push_u32(self.fields_per_row.len() as u32, &mut bp);
            for &f in &self.fields_per_row { bp.extend_from_slice(&f.to_le_bytes()); }

            let mut hasher = Sha256::new();
            hasher.update(&bp);
            let hash = hex::encode(hasher.finalize());

            let mut sb = Vec::with_capacity(self.schema_dict.len() * 32);
            push_u32(self.schema_dict.len() as u32, &mut sb);
            let mut sorted: Vec<(&Vec<u8>, &u16)> = self.schema_dict.iter().collect();
            sorted.sort_by_key(|&(_, &id)| id);
            for (k, &v) in sorted {
                push_u32(k.len() as u32, &mut sb);
                sb.extend_from_slice(k);
                sb.extend_from_slice(&v.to_le_bytes());
            }

            let mut p = Vec::with_capacity(64 + 4 + sb.len() + bp.len());
            p.extend_from_slice(hash.as_bytes());
            push_u32(sb.len() as u32, &mut p);
            p.extend_from_slice(&sb);
            p.extend_from_slice(&bp);
            (hash, p)
        };

        let mut zstd_buf = Vec::with_capacity(uncompressed_payload.len() / 3);
        let mut encoder = zstd::stream::Encoder::new(&mut zstd_buf, 9)?;
        let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        encoder.multithread(threads)?;
        encoder.write_all(&uncompressed_payload)?;
        encoder.finish()?;

        let mut out = Vec::with_capacity(HEADER_SIZE + zstd_buf.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION_MAJOR);
        out.push(VERSION_MINOR);
        out.extend_from_slice(&zstd_buf);
        Ok((out, hash_result))
    }

    pub fn decompress_buffer(payload: &[u8]) -> io::Result<Vec<u8>> {
        if payload.len() < HEADER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }
        if &payload[..4] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a LUMP file"));
        }
        if payload[4] != VERSION_MAJOR {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("unsupported version {}.{}, need {}.x", payload[4], payload[5], VERSION_MAJOR)));
        }

        let mut decoder = zstd::stream::Decoder::new(&payload[HEADER_SIZE..])?;

        let mut stored_hash_bytes = [0u8; 64];
        decoder.read_exact(&mut stored_hash_bytes)?;
        let stored_hash = std::str::from_utf8(&stored_hash_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid hash bytes"))?
            .to_owned();

        let mut flag = [0u8; 4];
        decoder.read_exact(&mut flag)?;
        let schema_len = u32::from_le_bytes(flag);

        if schema_len == 0xFFFFFFFF {
            let mut raw_out = Vec::new();
            decoder.read_to_end(&mut raw_out)?;
            let mut h = Sha256::new();
            h.update(&raw_out);
            if hex::encode(h.finalize()) != stored_hash {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "integrity check failed"));
            }
            return Ok(raw_out);
        }

        let mut schema_bytes = vec![0u8; schema_len as usize];
        decoder.read_exact(&mut schema_bytes)?;

        let mut bp = Vec::new();
        decoder.read_to_end(&mut bp)?;

        let mut h = Sha256::new();
        h.update(&bp);
        if hex::encode(h.finalize()) != stored_hash {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "integrity check failed"));
        }

        let r32 = |p: &[u8], c: &mut usize| -> u32 {
            let v = u32::from_le_bytes([p[*c], p[*c+1], p[*c+2], p[*c+3]]);
            *c += 4; v
        };

        let mut sc = 0usize;
        let num_keys = r32(&schema_bytes, &mut sc);
        let mut id_to_key: Vec<Vec<u8>> = vec![Vec::new(); num_keys as usize];
        for _ in 0..num_keys {
            let kl = r32(&schema_bytes, &mut sc) as usize;
            let kb = schema_bytes[sc..sc + kl].to_vec();
            sc += kl;
            let kid = u16::from_le_bytes([schema_bytes[sc], schema_bytes[sc + 1]]);
            sc += 2;
            id_to_key[kid as usize] = kb;
        }

        let mut c = 0usize;
        let dict_bytes_len = r32(&bp, &mut c) as usize;
        let dict_bytes_slice = &bp[c..c + dict_bytes_len];
        c += dict_bytes_len;

        let dl_len = r32(&bp, &mut c) as usize;
        let mut dict_lengths: Vec<u32> = Vec::with_capacity(dl_len);
        for _ in 0..dl_len { dict_lengths.push(r32(&bp, &mut c)); }

        let mut dict_lookups: Vec<&[u8]> = Vec::with_capacity(dl_len);
        let mut dc = 0usize;
        for &l in &dict_lengths {
            let ln = l as usize;
            dict_lookups.push(&dict_bytes_slice[dc..dc + ln]);
            dc += ln;
        }

        let keys_len = r32(&bp, &mut c) as usize;
        let mut keys_stream: Vec<u16> = Vec::with_capacity(keys_len);
        for _ in 0..keys_len {
            keys_stream.push(u16::from_le_bytes([bp[c], bp[c + 1]]));
            c += 2;
        }

        let types_len = r32(&bp, &mut c) as usize;
        let types_stream = &bp[c..c + types_len];
        c += types_len;

        let sids_len = r32(&bp, &mut c) as usize;
        let mut string_ids: Vec<u32> = Vec::with_capacity(sids_len);
        for _ in 0..sids_len { string_ids.push(r32(&bp, &mut c)); }

        let varint_len = r32(&bp, &mut c) as usize;
        let varint_stream = &bp[c..c + varint_len];
        c += varint_len;

        let lit_len = r32(&bp, &mut c) as usize;
        let literal_stream = &bp[c..c + lit_len];
        c += lit_len;

        let ll_count = r32(&bp, &mut c) as usize;
        let mut literal_lengths: Vec<u16> = Vec::with_capacity(ll_count);
        for _ in 0..ll_count {
            literal_lengths.push(u16::from_le_bytes([bp[c], bp[c + 1]]));
            c += 2;
        }

        let rows_len = r32(&bp, &mut c) as usize;
        let mut fields_per_row: Vec<u16> = Vec::with_capacity(rows_len);
        for _ in 0..rows_len {
            fields_per_row.push(u16::from_le_bytes([bp[c], bp[c + 1]]));
            c += 2;
        }

        let mut out = Vec::with_capacity(bp.len() * 8);
        let mut kc = 0usize;
        let mut sc2 = 0usize;
        let mut vc = 0usize;
        let mut lc = 0usize;
        let mut llc = 0usize;

        for &num_fields in &fields_per_row {
            out.push(b'{');
            for i in 0..num_fields {
                if i > 0 { out.extend_from_slice(b", "); }
                let key_id = keys_stream[kc] as usize;
                out.push(b'"');
                out.extend_from_slice(&id_to_key[key_id]);
                out.extend_from_slice(b"\": ");
                match types_stream[kc] {
                    TYPE_STRING => {
                        let vid = string_ids[sc2] as usize;
                        out.push(b'"');
                        out.extend_from_slice(dict_lookups[vid]);
                        out.push(b'"');
                        sc2 += 1;
                    }
                    TYPE_VARINT => {
                        let n = decode_zigzag_varint(varint_stream, &mut vc);
                        let s = n.to_string();
                        out.extend_from_slice(s.as_bytes());
                    }
                    TYPE_LITERAL => {
                        let ln = literal_lengths[llc] as usize;
                        out.extend_from_slice(&literal_stream[lc..lc + ln]);
                        lc += ln;
                        llc += 1;
                    }
                    TYPE_STRING_LIT => {
                        let ln = literal_lengths[llc] as usize;
                        out.push(b'"');
                        out.extend_from_slice(&literal_stream[lc..lc + ln]);
                        out.push(b'"');
                        lc += ln;
                        llc += 1;
                    }
                    _ => {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown type byte"));
                    }
                }
                kc += 1;
            }
            out.extend_from_slice(b"}\n");
        }

        Ok(out)
    }

    pub fn grep_buffer(payload: &[u8], key: &[u8], value: &[u8]) -> io::Result<Vec<Vec<u8>>> {
        if payload.len() < HEADER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too small"));
        }
        if &payload[..4] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a LUMP file"));
        }
        if payload[4] != VERSION_MAJOR {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("unsupported version {}.{}, need {}.x", payload[4], payload[5], VERSION_MAJOR)));
        }

        let mut decoder = zstd::stream::Decoder::new(&payload[HEADER_SIZE..])?;
        let mut stored_hash_bytes = [0u8; 64];
        decoder.read_exact(&mut stored_hash_bytes)?;
        let stored_hash = std::str::from_utf8(&stored_hash_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid hash bytes"))?
            .to_owned();
        let mut flag = [0u8; 4];
        decoder.read_exact(&mut flag)?;
        let schema_len = u32::from_le_bytes(flag);

        if schema_len == 0xFFFFFFFF {
            let mut raw_out = Vec::new();
            decoder.read_to_end(&mut raw_out)?;
            let mut h = Sha256::new();
            h.update(&raw_out);
            if hex::encode(h.finalize()) != stored_hash {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "integrity check failed"));
            }
            let mut matches = Vec::new();
            for line in raw_out.split(|&b| b == b'\n') {
                if line.is_empty() { continue; }
                if raw_line_matches(line, key, value) {
                    let mut m = line.to_vec();
                    m.push(b'\n');
                    matches.push(m);
                }
            }
            return Ok(matches);
        }

        let mut schema_bytes = vec![0u8; schema_len as usize];
        decoder.read_exact(&mut schema_bytes)?;
        let mut bp = Vec::new();
        decoder.read_to_end(&mut bp)?;

        let mut h = Sha256::new();
        h.update(&bp);
        if hex::encode(h.finalize()) != stored_hash {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "integrity check failed"));
        }

        let r32 = |p: &[u8], c: &mut usize| -> u32 {
            let v = u32::from_le_bytes([p[*c], p[*c+1], p[*c+2], p[*c+3]]);
            *c += 4; v
        };

        let mut sc = 0usize;
        let num_keys = r32(&schema_bytes, &mut sc);
        let mut id_to_key: Vec<Vec<u8>> = vec![Vec::new(); num_keys as usize];
        let mut target_key_id: Option<u16> = None;
        for _ in 0..num_keys {
            let kl = r32(&schema_bytes, &mut sc) as usize;
            let kb = schema_bytes[sc..sc + kl].to_vec();
            sc += kl;
            let kid = u16::from_le_bytes([schema_bytes[sc], schema_bytes[sc + 1]]);
            sc += 2;
            if kb.as_slice() == key { target_key_id = Some(kid); }
            id_to_key[kid as usize] = kb;
        }

        let Some(target_key_id) = target_key_id else { return Ok(Vec::new()); };

        let mut c = 0usize;
        let dict_bytes_len = r32(&bp, &mut c) as usize;
        let dict_bytes_slice = &bp[c..c + dict_bytes_len];
        c += dict_bytes_len;
        let dl_len = r32(&bp, &mut c) as usize;
        let mut dict_lengths: Vec<u32> = Vec::with_capacity(dl_len);
        for _ in 0..dl_len { dict_lengths.push(r32(&bp, &mut c)); }
        let mut dict_lookups: Vec<&[u8]> = Vec::with_capacity(dl_len);
        let mut dc = 0usize;
        for &l in &dict_lengths {
            let ln = l as usize;
            dict_lookups.push(&dict_bytes_slice[dc..dc + ln]);
            dc += ln;
        }

        let keys_len = r32(&bp, &mut c) as usize;
        let mut keys_stream: Vec<u16> = Vec::with_capacity(keys_len);
        for _ in 0..keys_len {
            keys_stream.push(u16::from_le_bytes([bp[c], bp[c + 1]]));
            c += 2;
        }
        let types_len = r32(&bp, &mut c) as usize;
        let types_stream = &bp[c..c + types_len];
        c += types_len;
        let sids_len = r32(&bp, &mut c) as usize;
        let mut string_ids: Vec<u32> = Vec::with_capacity(sids_len);
        for _ in 0..sids_len { string_ids.push(r32(&bp, &mut c)); }
        let varint_len = r32(&bp, &mut c) as usize;
        let varint_stream = &bp[c..c + varint_len];
        c += varint_len;
        let lit_len = r32(&bp, &mut c) as usize;
        let literal_stream = &bp[c..c + lit_len];
        c += lit_len;
        let ll_count = r32(&bp, &mut c) as usize;
        let mut literal_lengths: Vec<u16> = Vec::with_capacity(ll_count);
        for _ in 0..ll_count {
            literal_lengths.push(u16::from_le_bytes([bp[c], bp[c + 1]]));
            c += 2;
        }
        let rows_len = r32(&bp, &mut c) as usize;
        let mut fields_per_row: Vec<u16> = Vec::with_capacity(rows_len);
        for _ in 0..rows_len {
            fields_per_row.push(u16::from_le_bytes([bp[c], bp[c + 1]]));
            c += 2;
        }

        let target_int: Option<i64> = std::str::from_utf8(value).ok().and_then(|s| s.parse().ok());

        let mut matches = Vec::new();
        let mut row_buf: Vec<u8> = Vec::new();
        let mut kc = 0usize;
        let mut sc2 = 0usize;
        let mut vc = 0usize;
        let mut lc = 0usize;
        let mut llc = 0usize;

        for &num_fields in &fields_per_row {
            row_buf.clear();
            let mut matched = false;
            row_buf.push(b'{');
            for i in 0..num_fields {
                if i > 0 { row_buf.extend_from_slice(b", "); }
                let key_id = keys_stream[kc];
                let typ = types_stream[kc];
                kc += 1;
                row_buf.push(b'"');
                row_buf.extend_from_slice(&id_to_key[key_id as usize]);
                row_buf.extend_from_slice(b"\": ");
                match typ {
                    TYPE_STRING => {
                        let vid = string_ids[sc2] as usize;
                        sc2 += 1;
                        if key_id == target_key_id && dict_lookups[vid] == value {
                            matched = true;
                        }
                        row_buf.push(b'"');
                        row_buf.extend_from_slice(dict_lookups[vid]);
                        row_buf.push(b'"');
                    }
                    TYPE_VARINT => {
                        let n = decode_zigzag_varint(varint_stream, &mut vc);
                        if key_id == target_key_id {
                            if let Some(ti) = target_int {
                                if n == ti { matched = true; }
                            }
                        }
                        let s = n.to_string();
                        row_buf.extend_from_slice(s.as_bytes());
                    }
                    TYPE_LITERAL => {
                        let ln = literal_lengths[llc] as usize;
                        let lit = &literal_stream[lc..lc + ln];
                        if key_id == target_key_id && lit == value {
                            matched = true;
                        }
                        row_buf.extend_from_slice(lit);
                        lc += ln;
                        llc += 1;
                    }
                    TYPE_STRING_LIT => {
                        let ln = literal_lengths[llc] as usize;
                        let lit = &literal_stream[lc..lc + ln];
                        if key_id == target_key_id && lit == value {
                            matched = true;
                        }
                        row_buf.push(b'"');
                        row_buf.extend_from_slice(lit);
                        row_buf.push(b'"');
                        lc += ln;
                        llc += 1;
                    }
                    _ => {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "unknown type byte"));
                    }
                }
            }
            row_buf.extend_from_slice(b"}\n");
            if matched { matches.push(row_buf.clone()); }
        }

        Ok(matches)
    }
}
