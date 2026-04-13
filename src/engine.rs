use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write, BufWriter};
use sha2::{Sha256, Digest};
use memchr::memchr;

pub const MAGIC: &[u8; 4] = b"LUMP";
pub const VERSION_MAJOR: u8 = 0x06;
pub const VERSION_MINOR: u8 = 0x00;
const HEADER_SIZE: usize = 6;

fn trim_ascii_slice(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(s.len());
    let end = s.iter().rposition(|b| !b.is_ascii_whitespace()).map_or(start, |p| p + 1);
    &s[start..end]
}

fn is_number_byte(b: u8) -> bool {
    b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+' || b == b'e' || b == b'E'
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
    schema_dict: HashMap<Vec<u8>, u16>,
    next_key_id: u16,

    value_dict: HashMap<Vec<u8>, u32>,
    next_val_id: u32,
    dict_bytes: Vec<u8>,
    dict_lengths: Vec<u32>,

    keys_stream: Vec<u16>,
    types_stream: Vec<u8>,

    string_ids_stream: Vec<u32>,
    raw_numbers_stream: Vec<u8>,
    raw_numbers_lengths: Vec<u8>,

    fields_per_row: Vec<u16>,
}

#[derive(PartialEq)]
enum FsmState {
    ObjectStart,
    Key,
    Colon,
    ValueString,
    ValueNumber,
    ObjectEnd,
}

impl LumpiEngine {
    pub fn new() -> Self {
        LumpiEngine {
            schema_dict: HashMap::new(),
            next_key_id: 0,

            value_dict: HashMap::new(),
            next_val_id: 0,
            dict_bytes: Vec::with_capacity(2 * 1024 * 1024),
            dict_lengths: Vec::with_capacity(50 * 1024),

            keys_stream: Vec::with_capacity(5 * 1024 * 1024),
            types_stream: Vec::with_capacity(5 * 1024 * 1024),

            string_ids_stream: Vec::with_capacity(5 * 1024 * 1024),
            raw_numbers_stream: Vec::with_capacity(10 * 1024 * 1024),
            raw_numbers_lengths: Vec::with_capacity(5 * 1024 * 1024),

            fields_per_row: Vec::with_capacity(1024 * 1024),
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
        self.raw_numbers_stream.clear();
        self.raw_numbers_lengths.clear();
        self.fields_per_row.clear();
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
        if let Some(&id) = self.schema_dict.get(key) {
            id
        } else {
            let new_id = self.next_key_id;
            self.next_key_id += 1;
            self.schema_dict.insert(key.to_vec(), new_id);
            new_id
        }
    }

    fn intern_string_value(&mut self, val: &[u8]) -> u32 {
        if let Some(&id) = self.value_dict.get(val) {
            id
        } else {
            let new_id = self.next_val_id;
            self.next_val_id += 1;
            self.dict_bytes.extend_from_slice(val);
            self.dict_lengths.push(val.len() as u32);
            self.value_dict.insert(val.to_vec(), new_id);
            new_id
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
        for i in 0..=header_line.len() {
            if i == header_line.len() || header_line[i] == b',' {
                let mut col = trim_ascii_slice(&header_line[col_start..i]);
                if col.len() >= 2 && col[0] == b'"' && col[col.len() - 1] == b'"' {
                    col = &col[1..col.len() - 1];
                }
                headers.push(col);
                col_start = i + 1;
            }
        }

        if headers.is_empty() { return false; }

        let key_ids: Vec<u16> = headers.iter().map(|h| self.intern_key(h)).collect();

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

            if line.iter().all(|b| b.is_ascii_whitespace()) { continue; }

            let mut field_idx: usize = 0;
            let mut field_start = 0;

            for i in 0..=line.len() {
                if i == line.len() || line[i] == b',' {
                    if field_idx >= key_ids.len() { return false; }

                    let field = trim_ascii_slice(&line[field_start..i]);
                    self.keys_stream.push(key_ids[field_idx]);

                    if field.len() >= 2 && field[0] == b'"' && field[field.len() - 1] == b'"' {
                        let inner = &field[1..field.len() - 1];
                        self.types_stream.push(1);
                        let id = self.intern_string_value(inner);
                        self.string_ids_stream.push(id);
                    } else if !field.is_empty() && field.iter().all(|&b| is_number_byte(b)) {
                        self.types_stream.push(0);
                        self.raw_numbers_stream.extend_from_slice(field);
                        self.raw_numbers_lengths.push(field.len() as u8);
                    } else {
                        self.types_stream.push(1);
                        let id = self.intern_string_value(field);
                        self.string_ids_stream.push(id);
                    }

                    field_idx += 1;
                    field_start = i + 1;
                }
            }

            if field_idx > 0 {
                self.fields_per_row.push(field_idx as u16);
            }
        }

        !self.keys_stream.is_empty()
    }

    pub fn compress_buffer(&mut self, raw_data: &[u8]) -> io::Result<(Vec<u8>, String)> {
        let mut hasher = Sha256::new();
        hasher.update(raw_data);
        let hash_result = hex::encode(hasher.finalize());

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
            let mut val_start = 0;

            'parse: while cursor < len {
                match state {
                    FsmState::ObjectStart => {
                        if let Some(pos) = memchr(b'{', &parse_data[cursor..]) {
                            for &b in &parse_data[cursor..cursor + pos] {
                                if !b.is_ascii_whitespace() && b != b',' {
                                    is_structured = false;
                                    break 'parse;
                                }
                            }
                            cursor += pos + 1;
                            field_count = 0;
                            state = FsmState::Key;
                        } else {
                            break;
                        }
                    }
                    FsmState::Key => {
                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor >= len { break; }
                        if parse_data[cursor] == b'}' {
                            cursor += 1;
                            state = FsmState::ObjectEnd;
                            continue;
                        }
                        if parse_data[cursor] != b'"' { is_structured = false; break 'parse; }
                        cursor += 1;

                        let key_len = match memchr(b'"', &parse_data[cursor..]) {
                            Some(pos) => pos,
                            None => { is_structured = false; break 'parse; }
                        };
                        let key_slice = &parse_data[cursor..cursor + key_len];
                        cursor += key_len + 1;

                        let id = self.intern_key(key_slice);
                        self.keys_stream.push(id);
                        state = FsmState::Colon;
                    }
                    FsmState::Colon => {
                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor >= len || parse_data[cursor] != b':' { is_structured = false; break 'parse; }
                        cursor += 1;

                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor >= len { is_structured = false; break 'parse; }

                        if parse_data[cursor] == b'"' {
                            cursor += 1;
                            val_start = cursor;
                            state = FsmState::ValueString;
                        } else {
                            val_start = cursor;
                            state = FsmState::ValueNumber;
                        }
                    }
                    FsmState::ValueString => {
                        loop {
                            match memchr(b'"', &parse_data[cursor..]) {
                                Some(pos) => {
                                    cursor += pos;
                                    let mut escapes = 0;
                                    let mut check_idx = cursor - 1;
                                    while check_idx >= val_start && parse_data[check_idx] == b'\\' {
                                        escapes += 1;
                                        if check_idx == 0 { break; }
                                        check_idx -= 1;
                                    }
                                    if escapes % 2 == 1 {
                                        cursor += 1;
                                    } else {
                                        break;
                                    }
                                },
                                None => { is_structured = false; break 'parse; }
                            }
                        }
                        let val_slice = &parse_data[val_start..cursor];
                        cursor += 1;
                        self.types_stream.push(1);

                        let id = self.intern_string_value(val_slice);
                        self.string_ids_stream.push(id);
                        field_count += 1;

                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor < len && parse_data[cursor] == b',' {
                            cursor += 1;
                        }
                        state = FsmState::Key;
                    }
                    FsmState::ValueNumber => {
                        while cursor < len && parse_data[cursor] != b',' && parse_data[cursor] != b'}' && !parse_data[cursor].is_ascii_whitespace() {
                            cursor += 1;
                        }
                        let val_slice = &parse_data[val_start..cursor];
                        self.types_stream.push(0);
                        self.raw_numbers_stream.extend_from_slice(val_slice);
                        self.raw_numbers_lengths.push(val_slice.len() as u8);
                        field_count += 1;

                        while cursor < len && parse_data[cursor].is_ascii_whitespace() { cursor += 1; }
                        if cursor < len && parse_data[cursor] == b',' {
                            cursor += 1;
                        }
                        state = FsmState::Key;
                    }
                    FsmState::ObjectEnd => {
                        if field_count > 0 {
                            self.fields_per_row.push(field_count);
                        }
                        state = FsmState::ObjectStart;
                    }
                }
            }

            if state == FsmState::ObjectEnd && field_count > 0 {
                self.fields_per_row.push(field_count);
            }

            if self.keys_stream.is_empty() {
                is_structured = false;
            }
        }

        let mut uncompressed_payload = Vec::with_capacity(raw_data.len() / 2 + 1024);
        uncompressed_payload.extend_from_slice(hash_result.as_bytes());

        if !is_structured {
            uncompressed_payload.extend_from_slice(&0xFFFFFFFF_u32.to_le_bytes());
            uncompressed_payload.extend_from_slice(raw_data);
        } else {
            let mut block_payload = Vec::with_capacity(
                self.dict_bytes.len() +
                self.dict_lengths.len() * 4 +
                self.keys_stream.len() * 2 +
                self.types_stream.len() +
                self.string_ids_stream.len() * 4 +
                self.raw_numbers_stream.len() +
                self.raw_numbers_lengths.len() +
                self.fields_per_row.len() * 2 + 32
            );

            block_payload.extend_from_slice(&(self.dict_bytes.len() as u32).to_le_bytes());
            block_payload.extend_from_slice(&self.dict_bytes);
            block_payload.extend_from_slice(&(self.dict_lengths.len() as u32).to_le_bytes());
            for &l in &self.dict_lengths { block_payload.extend_from_slice(&l.to_le_bytes()); }
            block_payload.extend_from_slice(&(self.keys_stream.len() as u32).to_le_bytes());
            for &k in &self.keys_stream { block_payload.extend_from_slice(&k.to_le_bytes()); }
            block_payload.extend_from_slice(&(self.types_stream.len() as u32).to_le_bytes());
            block_payload.extend_from_slice(&self.types_stream);
            block_payload.extend_from_slice(&(self.string_ids_stream.len() as u32).to_le_bytes());
            for &v in &self.string_ids_stream { block_payload.extend_from_slice(&v.to_le_bytes()); }
            block_payload.extend_from_slice(&(self.raw_numbers_stream.len() as u32).to_le_bytes());
            block_payload.extend_from_slice(&self.raw_numbers_stream);
            block_payload.extend_from_slice(&(self.raw_numbers_lengths.len() as u32).to_le_bytes());
            block_payload.extend_from_slice(&self.raw_numbers_lengths);
            block_payload.extend_from_slice(&(self.fields_per_row.len() as u32).to_le_bytes());
            for &f in &self.fields_per_row { block_payload.extend_from_slice(&f.to_le_bytes()); }

            let mut schema_bytes = Vec::with_capacity(self.schema_dict.len() * 32);
            schema_bytes.extend_from_slice(&(self.schema_dict.len() as u32).to_le_bytes());

            let mut sorted_schema: Vec<(&Vec<u8>, &u16)> = self.schema_dict.iter().collect();
            sorted_schema.sort_by_key(|&(_, &id)| id);

            for (k, &v) in sorted_schema {
                schema_bytes.extend_from_slice(&(k.len() as u32).to_le_bytes());
                schema_bytes.extend_from_slice(k);
                schema_bytes.extend_from_slice(&v.to_le_bytes());
            }

            uncompressed_payload.extend_from_slice(&(schema_bytes.len() as u32).to_le_bytes());
            uncompressed_payload.extend_from_slice(&schema_bytes);
            uncompressed_payload.extend_from_slice(&block_payload);
        }

        let mut zstd_buffer = Vec::with_capacity(uncompressed_payload.len() / 3);
        let mut encoder = zstd::stream::Encoder::new(&mut zstd_buffer, 9)?;
        encoder.multithread(8)?;
        encoder.write_all(&uncompressed_payload)?;
        encoder.finish()?;

        let mut out_buffer = Vec::with_capacity(HEADER_SIZE + zstd_buffer.len());
        out_buffer.extend_from_slice(MAGIC);
        out_buffer.push(VERSION_MAJOR);
        out_buffer.push(VERSION_MINOR);
        out_buffer.extend_from_slice(&zstd_buffer);

        Ok((out_buffer, hash_result))
    }

    #[allow(dead_code)]
    pub fn compress(&mut self, input_path: &str, output_path: &str) -> io::Result<String> {
        let mut file = File::open(input_path)?;
        let mut raw_data = Vec::new();
        file.read_to_end(&mut raw_data)?;
        let (out_buffer, hash_result) = self.compress_buffer(&raw_data)?;
        let mut out_file = File::create(output_path)?;
        out_file.write_all(&out_buffer)?;
        Ok(hash_result)
    }

    pub fn decompress(input_path: &str, output_path: &str) -> io::Result<bool> {
        let mut in_file = File::open(input_path)?;

        let mut header = [0u8; HEADER_SIZE];
        in_file.read_exact(&mut header)?;
        if &header[..4] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Not a valid LUMP file"));
        }

        let mut decoder = zstd::stream::Decoder::new(in_file)?;

        let mut stored_hash_bytes = [0u8; 64];
        decoder.read_exact(&mut stored_hash_bytes)?;
        let stored_hash = std::str::from_utf8(&stored_hash_bytes).unwrap();

        let mut schema_len_bytes = [0u8; 4];
        decoder.read_exact(&mut schema_len_bytes)?;
        let schema_len = u32::from_le_bytes(schema_len_bytes);

        if schema_len == 0xFFFFFFFF {
            let mut out_file = File::create(output_path)?;
            io::copy(&mut decoder, &mut out_file)?;

            let mut check_file = File::open(output_path)?;
            let mut check_data = Vec::new();
            check_file.read_to_end(&mut check_data)?;
            let mut hasher = Sha256::new();
            hasher.update(&check_data);
            return Ok(hex::encode(hasher.finalize()) == stored_hash);
        }

        let mut schema_bytes = vec![0u8; schema_len as usize];
        decoder.read_exact(&mut schema_bytes)?;

        let mut s_cursor = 0;
        let read_u32_s = |p: &[u8], c: &mut usize| -> u32 {
            let val = u32::from_le_bytes([p[*c], p[*c+1], p[*c+2], p[*c+3]]);
            *c += 4; val
        };
        let num_keys = read_u32_s(&schema_bytes, &mut s_cursor);
        let mut id_to_key: Vec<Vec<u8>> = vec![Vec::new(); num_keys as usize];
        for _ in 0..num_keys {
            let k_len = read_u32_s(&schema_bytes, &mut s_cursor) as usize;
            let k_bytes = schema_bytes[s_cursor..s_cursor+k_len].to_vec();
            s_cursor += k_len;
            let k_id = u16::from_le_bytes([schema_bytes[s_cursor], schema_bytes[s_cursor+1]]);
            s_cursor += 2;
            id_to_key[k_id as usize] = k_bytes;
        }

        let mut payload = Vec::new();
        decoder.read_to_end(&mut payload)?;

        let mut cursor = 0;
        let read_u32 = |p: &[u8], c: &mut usize| -> u32 {
            let val = u32::from_le_bytes([p[*c], p[*c+1], p[*c+2], p[*c+3]]);
            *c += 4; val
        };

        let dict_bytes_len = read_u32(&payload, &mut cursor) as usize;
        let dict_bytes = &payload[cursor..cursor+dict_bytes_len];
        cursor += dict_bytes_len;

        let dict_lengths_len = read_u32(&payload, &mut cursor) as usize;
        let mut dict_lengths = Vec::with_capacity(dict_lengths_len);
        for _ in 0..dict_lengths_len { dict_lengths.push(read_u32(&payload, &mut cursor)); }

        let mut dict_lookups = Vec::with_capacity(dict_lengths_len);
        let mut dict_cursor = 0;
        for &len in &dict_lengths {
            let l = len as usize;
            dict_lookups.push(&dict_bytes[dict_cursor..dict_cursor+l]);
            dict_cursor += l;
        }

        let keys_len = read_u32(&payload, &mut cursor) as usize;
        let mut keys_stream = Vec::with_capacity(keys_len);
        for _ in 0..keys_len {
            keys_stream.push(u16::from_le_bytes([payload[cursor], payload[cursor+1]]));
            cursor += 2;
        }

        let types_len = read_u32(&payload, &mut cursor) as usize;
        let types_stream = &payload[cursor..cursor+types_len];
        cursor += types_len;

        let string_ids_len = read_u32(&payload, &mut cursor) as usize;
        let mut string_ids_stream = Vec::with_capacity(string_ids_len);
        for _ in 0..string_ids_len { string_ids_stream.push(read_u32(&payload, &mut cursor)); }

        let raw_numbers_len = read_u32(&payload, &mut cursor) as usize;
        let raw_numbers_stream = &payload[cursor..cursor+raw_numbers_len];
        cursor += raw_numbers_len;

        let raw_numbers_lengths_len = read_u32(&payload, &mut cursor) as usize;
        let raw_numbers_lengths = &payload[cursor..cursor+raw_numbers_lengths_len];
        cursor += raw_numbers_lengths_len;

        let rows_len = read_u32(&payload, &mut cursor) as usize;
        let mut fields_per_row = Vec::with_capacity(rows_len);
        for _ in 0..rows_len {
            fields_per_row.push(u16::from_le_bytes([payload[cursor], payload[cursor+1]]));
            cursor += 2;
        }

        let out_file = File::create(output_path)?;
        let mut writer = BufWriter::new(out_file);

        let mut key_cursor = 0;
        let mut string_cursor = 0;
        let mut num_bytes_cursor = 0;
        let mut num_len_cursor = 0;

        for &num_fields in &fields_per_row {
            let _ = writer.write_all(b"{");

            for i in 0..num_fields {
                if i > 0 { let _ = writer.write_all(b", "); }
                let key_id = keys_stream[key_cursor];
                let _ = writer.write_all(b"\"");
                let _ = writer.write_all(&id_to_key[key_id as usize]);
                let _ = writer.write_all(b"\": ");
                let val_type = types_stream[key_cursor];

                if val_type == 1 {
                    let val_id = string_ids_stream[string_cursor] as usize;
                    let _ = writer.write_all(b"\"");
                    let _ = writer.write_all(dict_lookups[val_id]);
                    let _ = writer.write_all(b"\"");
                    string_cursor += 1;
                } else {
                    let len = raw_numbers_lengths[num_len_cursor] as usize;
                    let _ = writer.write_all(&raw_numbers_stream[num_bytes_cursor..num_bytes_cursor+len]);
                    num_bytes_cursor += len;
                    num_len_cursor += 1;
                }
                key_cursor += 1;
            }
            let _ = writer.write_all(b"}\n");
        }
        let _ = writer.flush();

        let mut check_file = File::open(output_path)?;
        let mut check_data = Vec::new();
        check_file.read_to_end(&mut check_data)?;
        let mut hasher = Sha256::new();
        hasher.update(&check_data);
        Ok(hex::encode(hasher.finalize()) == stored_hash)
    }

    pub fn decompress_buffer(payload: &[u8]) -> io::Result<Vec<u8>> {
        if payload.len() < HEADER_SIZE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "File too small"));
        }
        if &payload[..4] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Not a valid LUMP file"));
        }

        let zstd_data = &payload[HEADER_SIZE..];
        let mut decoder = zstd::stream::Decoder::new(zstd_data)?;

        let mut stored_hash_bytes = [0u8; 64];
        decoder.read_exact(&mut stored_hash_bytes)?;

        let mut schema_len_bytes = [0u8; 4];
        decoder.read_exact(&mut schema_len_bytes)?;
        let schema_len = u32::from_le_bytes(schema_len_bytes);

        if schema_len == 0xFFFFFFFF {
            let mut raw_out = Vec::new();
            decoder.read_to_end(&mut raw_out)?;
            return Ok(raw_out);
        }

        let mut schema_bytes = vec![0u8; schema_len as usize];
        decoder.read_exact(&mut schema_bytes)?;

        let mut s_cursor = 0;
        let read_u32_s = |p: &[u8], c: &mut usize| -> u32 {
            let val = u32::from_le_bytes([p[*c], p[*c+1], p[*c+2], p[*c+3]]);
            *c += 4; val
        };
        let num_keys = read_u32_s(&schema_bytes, &mut s_cursor);
        let mut id_to_key: Vec<Vec<u8>> = vec![Vec::new(); num_keys as usize];
        for _ in 0..num_keys {
            let k_len = read_u32_s(&schema_bytes, &mut s_cursor) as usize;
            let k_bytes = schema_bytes[s_cursor..s_cursor+k_len].to_vec();
            s_cursor += k_len;
            let k_id = u16::from_le_bytes([schema_bytes[s_cursor], schema_bytes[s_cursor+1]]);
            s_cursor += 2;
            id_to_key[k_id as usize] = k_bytes;
        }

        let mut inner_payload = Vec::new();
        decoder.read_to_end(&mut inner_payload)?;

        let mut cursor = 0;
        let read_u32 = |p: &[u8], c: &mut usize| -> u32 {
            let val = u32::from_le_bytes([p[*c], p[*c+1], p[*c+2], p[*c+3]]);
            *c += 4; val
        };

        let dict_bytes_len = read_u32(&inner_payload, &mut cursor) as usize;
        let dict_bytes = &inner_payload[cursor..cursor+dict_bytes_len];
        cursor += dict_bytes_len;

        let dict_lengths_len = read_u32(&inner_payload, &mut cursor) as usize;
        let mut dict_lengths = Vec::with_capacity(dict_lengths_len);
        for _ in 0..dict_lengths_len { dict_lengths.push(read_u32(&inner_payload, &mut cursor)); }

        let mut dict_lookups = Vec::with_capacity(dict_lengths_len);
        let mut dict_cursor = 0;
        for &len in &dict_lengths {
            let l = len as usize;
            dict_lookups.push(&dict_bytes[dict_cursor..dict_cursor+l]);
            dict_cursor += l;
        }

        let keys_len = read_u32(&inner_payload, &mut cursor) as usize;
        let mut keys_stream = Vec::with_capacity(keys_len);
        for _ in 0..keys_len {
            keys_stream.push(u16::from_le_bytes([inner_payload[cursor], inner_payload[cursor+1]]));
            cursor += 2;
        }

        let types_len = read_u32(&inner_payload, &mut cursor) as usize;
        let types_stream = &inner_payload[cursor..cursor+types_len];
        cursor += types_len;

        let string_ids_len = read_u32(&inner_payload, &mut cursor) as usize;
        let mut string_ids_stream = Vec::with_capacity(string_ids_len);
        for _ in 0..string_ids_len { string_ids_stream.push(read_u32(&inner_payload, &mut cursor)); }

        let raw_numbers_len = read_u32(&inner_payload, &mut cursor) as usize;
        let raw_numbers_stream = &inner_payload[cursor..cursor+raw_numbers_len];
        cursor += raw_numbers_len;

        let raw_numbers_lengths_len = read_u32(&inner_payload, &mut cursor) as usize;
        let raw_numbers_lengths = &inner_payload[cursor..cursor+raw_numbers_lengths_len];
        cursor += raw_numbers_lengths_len;

        let rows_len = read_u32(&inner_payload, &mut cursor) as usize;
        let mut fields_per_row = Vec::with_capacity(rows_len);
        for _ in 0..rows_len {
            fields_per_row.push(u16::from_le_bytes([inner_payload[cursor], inner_payload[cursor+1]]));
            cursor += 2;
        }

        let mut out_buffer = Vec::with_capacity(inner_payload.len() * 10);
        let mut key_cursor = 0;
        let mut string_cursor = 0;
        let mut num_bytes_cursor = 0;
        let mut num_len_cursor = 0;

        for &num_fields in &fields_per_row {
            let _ = out_buffer.write_all(b"{");
            for i in 0..num_fields {
                if i > 0 { let _ = out_buffer.write_all(b", "); }
                let key_id = keys_stream[key_cursor];
                let _ = out_buffer.write_all(b"\"");
                let _ = out_buffer.write_all(&id_to_key[key_id as usize]);
                let _ = out_buffer.write_all(b"\": ");
                let val_type = types_stream[key_cursor];

                if val_type == 1 {
                    let val_id = string_ids_stream[string_cursor] as usize;
                    let _ = out_buffer.write_all(b"\"");
                    let _ = out_buffer.write_all(dict_lookups[val_id]);
                    let _ = out_buffer.write_all(b"\"");
                    string_cursor += 1;
                } else {
                    let len = raw_numbers_lengths[num_len_cursor] as usize;
                    let _ = out_buffer.write_all(&raw_numbers_stream[num_bytes_cursor..num_bytes_cursor+len]);
                    num_bytes_cursor += len;
                    num_len_cursor += 1;
                }
                key_cursor += 1;
            }
            let _ = out_buffer.write_all(b"}\n");
        }

        Ok(out_buffer)
    }
}
