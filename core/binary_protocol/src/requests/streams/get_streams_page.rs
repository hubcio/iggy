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

use crate::codec::{WireDecode, WireEncode, read_u32_le};
use crate::primitives::metadata_changes::{
    optional_u32_size, put_flag, put_optional_u32, read_flag, read_optional_u32, read_wire_name,
};
use crate::{WireError, WireName};
use bytes::{BufMut, BytesMut};
use std::borrow::Cow;

/// `GetStreamsPage` request: the streams after `after_id`, in ascending id.
///
/// Wire format:
/// ```text
/// [has_after:u8][after_id:u32 if 1]
/// [limit:u32]
/// [has_name:u8][name_len:u8][name if 1]
/// ```
///
/// The decode checks the structure only. The server checks the limit range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetStreamsPageRequest {
    /// The `next_after_id` of the previous page, `None` for the first page.
    pub after_id: Option<u32>,
    /// The most streams the page returns.
    pub limit: u32,
    /// Only the streams whose name contains this text, case-sensitive.
    pub name: Option<WireName>,
}

impl WireEncode for GetStreamsPageRequest {
    fn encoded_size(&self) -> usize {
        optional_u32_size(self.after_id)
            + 4
            + 1
            + self.name.as_ref().map_or(0, WireEncode::encoded_size)
    }

    fn encode(&self, buf: &mut BytesMut) {
        put_optional_u32(buf, self.after_id);
        buf.put_u32_le(self.limit);
        put_flag(buf, self.name.is_some());
        if let Some(name) = &self.name {
            name.encode(buf);
        }
    }
}

impl WireDecode for GetStreamsPageRequest {
    fn decode(buf: &[u8]) -> Result<(Self, usize), WireError> {
        let (after_id, mut position) = read_optional_u32(buf, 0)?;
        let limit = read_u32_le(buf, position)?;
        position += 4;
        let has_name = read_flag(buf, position)?;
        position += 1;
        let name = if has_name {
            let (name, consumed) = read_wire_name(buf, position)?;
            position += consumed;
            Some(name)
        } else {
            None
        };
        if position != buf.len() {
            return Err(WireError::Validation(Cow::Owned(format!(
                "{} trailing bytes after a streams page request",
                buf.len() - position
            ))));
        }
        Ok((
            Self {
                after_id,
                limit,
                name,
            },
            position,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered() -> GetStreamsPageRequest {
        GetStreamsPageRequest {
            after_id: Some(41),
            limit: 100,
            name: Some(WireName::new("orders").unwrap()),
        }
    }

    #[test]
    fn given_a_first_page_without_a_name_when_round_tripping_should_keep_it() {
        let request = GetStreamsPageRequest {
            after_id: None,
            limit: 1,
            name: None,
        };
        let bytes = request.to_bytes();
        assert_eq!(bytes.len(), request.encoded_size());
        assert_eq!(&bytes[..], &[0, 1, 0, 0, 0, 0]);
        assert_eq!(GetStreamsPageRequest::decode(&bytes).unwrap(), (request, 6));
    }

    #[test]
    fn given_a_cursor_and_a_name_when_round_tripping_should_keep_both() {
        let request = filtered();
        let bytes = request.to_bytes();
        assert_eq!(bytes.len(), request.encoded_size());
        let (decoded, consumed) = GetStreamsPageRequest::decode(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, request);
    }

    #[test]
    fn given_trailing_bytes_when_decoding_should_refuse() {
        let mut bytes = filtered().to_bytes().to_vec();
        bytes.push(0);
        assert!(GetStreamsPageRequest::decode(&bytes).is_err());
    }

    #[test]
    fn given_a_flag_other_than_zero_or_one_when_decoding_should_refuse() {
        let mut bytes = filtered().to_bytes().to_vec();
        bytes[0] = 2;
        assert!(GetStreamsPageRequest::decode(&bytes).is_err());
        let mut bytes = filtered().to_bytes().to_vec();
        bytes[9] = 2;
        assert!(GetStreamsPageRequest::decode(&bytes).is_err());
    }

    #[test]
    fn given_an_empty_name_when_decoding_should_refuse() {
        let request = GetStreamsPageRequest {
            name: None,
            ..filtered()
        };
        let mut bytes = request.to_bytes().to_vec();
        *bytes.last_mut().unwrap() = 1;
        bytes.push(0);
        assert!(GetStreamsPageRequest::decode(&bytes).is_err());
    }

    #[test]
    fn given_a_truncated_request_when_decoding_should_refuse() {
        let bytes = filtered().to_bytes();
        for length in 0..bytes.len() {
            assert!(
                GetStreamsPageRequest::decode(&bytes[..length]).is_err(),
                "a request cut at {length} bytes must not decode"
            );
        }
    }
}
