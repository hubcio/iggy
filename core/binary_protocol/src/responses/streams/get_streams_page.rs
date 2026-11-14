// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use crate::WireError;
use crate::codec::{WireDecode, WireEncode, read_u32_le};
use crate::primitives::metadata_changes::{optional_u32_size, put_optional_u32, read_optional_u32};
use crate::responses::streams::StreamResponse;
use bytes::{BufMut, BytesMut};
use std::borrow::Cow;

/// `GetStreamsPage` response: one page of streams in ascending id.
///
/// Wire format:
/// ```text
/// [has_next:u8][next_after_id:u32 if 1]
/// [count:u32]
/// [StreamResponse]*count
/// ```
///
/// `next_after_id` is set while more streams may follow, and the next request
/// passes it as `after_id`. The server scans a bounded number of streams per
/// page, so a page can hold fewer streams than the limit, or none, while
/// `next_after_id` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetStreamsPageResponse {
    pub streams: Vec<StreamResponse>,
    pub next_after_id: Option<u32>,
}

impl WireEncode for GetStreamsPageResponse {
    fn encoded_size(&self) -> usize {
        optional_u32_size(self.next_after_id)
            + 4
            + self
                .streams
                .iter()
                .map(WireEncode::encoded_size)
                .sum::<usize>()
    }

    fn encode(&self, buf: &mut BytesMut) {
        put_optional_u32(buf, self.next_after_id);
        // A page holds at most the page limit, a `u32`.
        #[allow(clippy::cast_possible_truncation)]
        buf.put_u32_le(self.streams.len() as u32);
        for stream in &self.streams {
            stream.encode(buf);
        }
    }
}

impl WireDecode for GetStreamsPageResponse {
    fn decode(buf: &[u8]) -> Result<(Self, usize), WireError> {
        let (next_after_id, mut position) = read_optional_u32(buf, 0)?;
        let count = read_u32_le(buf, position)? as usize;
        position += 4;
        // Each record takes more than one byte, so the buffer bounds the
        // allocation whatever count a peer claims.
        let mut streams = Vec::with_capacity(count.min(buf.len().saturating_sub(position)));
        for _ in 0..count {
            let (stream, consumed) = StreamResponse::decode(&buf[position..])?;
            position += consumed;
            streams.push(stream);
        }
        if position != buf.len() {
            return Err(WireError::Validation(Cow::Owned(format!(
                "{} trailing bytes after a streams page",
                buf.len() - position
            ))));
        }
        Ok((
            Self {
                streams,
                next_after_id,
            },
            position,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WireName, WireOptions};

    fn stream(id: u32, name: &str) -> StreamResponse {
        StreamResponse {
            id,
            created_at: u64::from(id) * 100,
            topics_count: 1,
            size_bytes: 512,
            messages_count: 50,
            name: WireName::new(name).unwrap(),
            options: WireOptions::empty(),
            derived_options: WireOptions::empty(),
        }
    }

    fn page() -> GetStreamsPageResponse {
        GetStreamsPageResponse {
            streams: vec![stream(3, "orders-eu"), stream(7, "orders-us")],
            next_after_id: Some(7),
        }
    }

    #[test]
    fn given_the_last_empty_page_when_round_tripping_should_keep_it() {
        let response = GetStreamsPageResponse {
            streams: Vec::new(),
            next_after_id: None,
        };
        let bytes = response.to_bytes();
        assert_eq!(&bytes[..], &[0, 0, 0, 0, 0]);
        assert_eq!(
            GetStreamsPageResponse::decode(&bytes).unwrap(),
            (response, 5)
        );
    }

    #[test]
    fn given_streams_and_a_cursor_when_round_tripping_should_keep_both() {
        let response = page();
        let bytes = response.to_bytes();
        assert_eq!(bytes.len(), response.encoded_size());
        let (decoded, consumed) = GetStreamsPageResponse::decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, response);
    }

    #[test]
    fn given_trailing_bytes_when_decoding_should_refuse() {
        let mut bytes = page().to_bytes().to_vec();
        bytes.push(0);
        assert!(GetStreamsPageResponse::decode(&bytes).is_err());
    }

    #[test]
    fn given_a_count_above_the_records_when_decoding_should_refuse() {
        let mut bytes = page().to_bytes().to_vec();
        bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(GetStreamsPageResponse::decode(&bytes).is_err());
    }
}
