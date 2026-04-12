use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write, BufWriter};
use sha2::{Sha256, Digest};
use memchr::memchr;

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

    pub fn compress_buffer(&mut self, raw_data: &[u8]) -> io::Result<(Vec<u8>, String)> {
        let mut hasher = Sha256::new();
        hasher.update(raw_data);
        let hash_result = hex::encode(hasher.finalize());

        let len = raw_data.len();
        let mut cursor = 0;
        let mut is_pure_jsonl = true;

        'parse: while cursor < len {
            let next_brace = match memchr(b'{', &raw_data[cursor..]) {
                Some(pos) => pos,
                None => break,
            };

            for &b in &raw_data[cursor..cursor + next_brace] {
                if !b.is_ascii_whitespace() {
                    is_pure_jsonl = false; break 'parse;
                }
            }
            cursor += next_brace + 1;

            let mut field_count = 0;

            loop {
                while cursor < len && raw_data[cursor].is_ascii_whitespace() { 
                    cursor += 1; 
                }
                if cursor >= len { break; }

                if raw_data[cursor] == b'}' {
                    cursor += 1;
                    break;
                }

                if raw_data[cursor] != b'"' {
                    is_pure_jsonl = false; break 'parse;
                }
                cursor += 1;

                let key_start = cursor;
                let key_len = match memchr(b'"', &raw_data[cursor..]) {
                    Some(pos) => pos,
                    None => { is_pure_jsonl = false; break 'parse; }
                };
                let key_slice = &raw_data[cursor..cursor + key_len];
                cursor += key_len + 1;

                if let Some(&id) = self.schema_dict.get(key_slice) {
                    self.keys_stream.push(id);
                } else {
                    let id = self.next_key_id;
                    self.next_key_id += 1;
                    self.schema_dict.insert(key_slice.to_vec(), id);
                    self.keys_stream.push(id);
                }

                while cursor < len && raw_data[cursor].is_ascii_whitespace() { 
                    cursor += 1; 
                }
                if cursor >= len || raw_data[cursor] != b':' {
                    is_pure_jsonl = false; break 'parse;
                }
                cursor += 1;

                while cursor < len && raw_data[cursor].is_ascii_whitespace() { 
                    cursor += 1; 
                }
                if cursor >= len { is_pure_jsonl = false; break 'parse; }

                let is_string = raw_data[cursor] == b'"';
                let val_start;
                let val_end;

                if is_string {
                    cursor += 1;
                    val_start = cursor;
                    loop {
                        match memchr(b'"', &raw_data[cursor..]) {
                            Some(pos) => {
                                cursor += pos;
                                let mut escapes = 0;
                                let mut check_idx = cursor - 1;
                                while check_idx >= val_start && raw_data[check_idx] == b'\\' {
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
                            None => { is_pure_jsonl = false; break 'parse; }
                        }
                    }
                    val_end = cursor;
                    cursor += 1;
                } else {
                    val_start = cursor;
                    while cursor < len && raw_data[cursor] != b',' && raw_data[cursor] != b'}' && !raw_data[cursor].is_ascii_whitespace() {
                        cursor += 1;
                    }
                    val_end = cursor;
                }

                let val_slice = &raw_data[val_start..val_end];
                self.types_stream.push(if is_string { 1 } else { 0 });

                if is_string {
                    if let Some(&id) = self.value_dict.get(val_slice) {
                        self.string_ids_stream.push(id);
                    } else {
                        let id = self.next_val_id;
                        self.next_val_id += 1;
                        self.dict_bytes.extend_from_slice(val_slice);
                        self.dict_lengths.push(val_slice.len() as u32);
                        self.value_dict.insert(val_slice.to_vec(), id);
                        self.string_ids_stream.push(id);
                    }
                } else {
                    self.raw_numbers_stream.extend_from_slice(val_slice);
                    self.raw_numbers_lengths.push(val_slice.len() as u8);
                }

                field_count += 1;

                while cursor < len && raw_data[cursor].is_ascii_whitespace() { 
                    cursor += 1; 
                }
                if cursor < len && raw_data[cursor] == b',' {
                    cursor += 1;
                }
            }
            if field_count > 0 {
                self.fields_per_row.push(field_count);
            }
        }

        let mut out_buffer = Vec::with_capacity(raw_data.len() / 3);
        let mut encoder = if raw_data.len() < 50 * 1024 * 1024 {
            zstd::stream::Encoder::new(&mut out_buffer, 9)?
        } else {
            let mut enc = zstd::stream::Encoder::new(&mut out_buffer, 11)?;
            enc.multithread(8)?;
            enc
        };

        if !is_pure_jsonl || self.keys_stream.is_empty() {
            encoder.write_all(hash_result.as_bytes())?; 
            encoder.write_all(&0xFFFFFFFF_u32.to_le_bytes())?;
            encoder.write_all(raw_data)?;
            encoder.finish()?;
            return Ok((out_buffer, hash_result));
        }

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
        for (k, &v) in &self.schema_dict {
            schema_bytes.extend_from_slice(&(k.len() as u32).to_le_bytes());
            schema_bytes.extend_from_slice(k);
            schema_bytes.extend_from_slice(&v.to_le_bytes());
        }

        encoder.write_all(hash_result.as_bytes())?; 
        encoder.write_all(&(schema_bytes.len() as u32).to_le_bytes())?;
        encoder.write_all(&schema_bytes)?;
        encoder.write_all(&block_payload)?;
        encoder.finish()?;

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
        let in_file = File::open(input_path)?;
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
        let mut decoder = zstd::stream::Decoder::new(payload)?;
        
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