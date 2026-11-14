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

use crate::types::metadata_changes::query::parse_canonical;
use crate::{IggyError, Stream};
use iggy_binary_protocol::WireName;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Streams one page returns when the query names no limit.
pub const DEFAULT_STREAMS_PAGE_LIMIT: u32 = 100;

/// The most streams one page returns.
pub const MAX_STREAMS_PAGE_LIMIT: u32 = 1_000;

const AFTER_ID_KEY: &str = "after_id";
const LIMIT_KEY: &str = "limit";
const NAME_KEY: &str = "name";

/// One request of a walk over the streams in ascending id.
///
/// A new stream takes the lowest free id, so a stream created during a walk
/// can land behind the cursor and miss that walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamsPageQuery {
    /// The `next_after_id` of the previous page, `None` for the first page.
    pub after_id: Option<u32>,
    /// The most streams the page returns, 1 to [`MAX_STREAMS_PAGE_LIMIT`].
    pub limit: u32,
    /// Only the streams whose name contains this text, case-sensitive.
    pub name: Option<String>,
}

impl Default for StreamsPageQuery {
    fn default() -> Self {
        Self {
            after_id: None,
            limit: DEFAULT_STREAMS_PAGE_LIMIT,
            name: None,
        }
    }
}

impl StreamsPageQuery {
    /// The query of the page after `page`, `None` after the last page.
    #[must_use]
    pub fn next(&self, page: &StreamsPage) -> Option<Self> {
        page.next_after_id.map(|after_id| Self {
            after_id: Some(after_id),
            ..self.clone()
        })
    }

    /// Check the rules every transport shares.
    ///
    /// # Errors
    ///
    /// `InvalidCommand` for a limit outside 1 to [`MAX_STREAMS_PAGE_LIMIT`], or
    /// a name that is empty or longer than a stream name can be.
    pub fn validate(&self) -> Result<(), IggyError> {
        if !(1..=MAX_STREAMS_PAGE_LIMIT).contains(&self.limit) {
            return Err(IggyError::InvalidCommand);
        }
        self.wire_name().map(|_| ())
    }

    /// The name filter as the binary request carries it.
    ///
    /// # Errors
    ///
    /// `InvalidCommand` for a name that is empty or longer than 255 bytes.
    pub fn wire_name(&self) -> Result<Option<WireName>, IggyError> {
        self.name
            .as_deref()
            .map(WireName::new)
            .transpose()
            .map_err(|_| IggyError::InvalidCommand)
    }

    /// The HTTP query parameters for this query, in a stable order.
    #[must_use]
    pub fn http_query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = Vec::with_capacity(3);
        if let Some(after_id) = self.after_id {
            pairs.push((AFTER_ID_KEY, after_id.to_string()));
        }
        pairs.push((LIMIT_KEY, self.limit.to_string()));
        if let Some(name) = &self.name {
            pairs.push((NAME_KEY, name.clone()));
        }
        pairs
    }

    /// Parse the HTTP query parameters. An unknown, repeated or malformed
    /// parameter is refused, so a typo cannot silently read the first page.
    ///
    /// # Errors
    ///
    /// `InvalidCommand` for any refused parameter.
    pub fn from_http_query<'a>(
        pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, IggyError> {
        let mut seen = BTreeSet::new();
        let mut query = Self::default();
        for (key, value) in pairs {
            if !seen.insert(key) {
                return Err(IggyError::InvalidCommand);
            }
            match key {
                AFTER_ID_KEY => query.after_id = Some(parse_canonical(value)?),
                LIMIT_KEY => query.limit = parse_canonical(value)?,
                NAME_KEY => query.name = Some(value.to_owned()),
                _ => return Err(IggyError::InvalidCommand),
            }
        }
        query.validate()?;
        Ok(query)
    }
}

/// One page of streams in ascending id.
#[derive(Debug, Serialize, Deserialize)]
pub struct StreamsPage {
    pub streams: Vec<Stream>,
    /// Set while more streams may follow: the next query passes it as
    /// `after_id`. The server scans a bounded number of streams per page, so
    /// a page can hold fewer streams than the limit, or none, while it is set.
    pub next_after_id: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(query: &StreamsPageQuery) -> Vec<(&'static str, String)> {
        query.http_query_pairs()
    }

    fn parse(pairs: &[(&str, &str)]) -> Result<StreamsPageQuery, IggyError> {
        StreamsPageQuery::from_http_query(pairs.iter().copied())
    }

    #[test]
    fn given_a_query_when_round_tripping_http_pairs_should_keep_it() {
        for query in [
            StreamsPageQuery::default(),
            StreamsPageQuery {
                after_id: Some(41),
                limit: MAX_STREAMS_PAGE_LIMIT,
                name: Some("orders eu/1".to_owned()),
            },
        ] {
            let pairs = pairs(&query);
            let parsed = StreamsPageQuery::from_http_query(
                pairs.iter().map(|(key, value)| (*key, value.as_str())),
            )
            .unwrap();
            assert_eq!(parsed, query);
        }
    }

    #[test]
    fn given_no_parameters_when_parsing_should_read_the_first_default_page() {
        assert_eq!(parse(&[]).unwrap(), StreamsPageQuery::default());
    }

    #[test]
    fn given_refused_parameters_when_parsing_should_reject_them() {
        for refused in [
            &[("after", "1")][..],
            &[("limit", "10"), ("limit", "10")],
            &[("limit", "0")],
            &[("limit", "1001")],
            &[("limit", "010")],
            &[("limit", "+5")],
            &[("after_id", "-1")],
            &[("after_id", "4294967296")],
            &[("name", "")],
        ] {
            assert_eq!(
                parse(refused),
                Err(IggyError::InvalidCommand),
                "{refused:?} must be refused"
            );
        }
    }

    #[test]
    fn given_a_name_longer_than_a_stream_name_when_validating_should_refuse() {
        let query = StreamsPageQuery {
            name: Some("n".repeat(256)),
            ..StreamsPageQuery::default()
        };
        assert_eq!(query.validate(), Err(IggyError::InvalidCommand));
        let query = StreamsPageQuery {
            name: Some("n".repeat(255)),
            ..StreamsPageQuery::default()
        };
        assert_eq!(query.validate(), Ok(()));
    }

    #[test]
    fn given_a_page_when_asking_for_the_next_query_should_continue_after_its_cursor() {
        let query = StreamsPageQuery {
            name: Some("orders".to_owned()),
            ..StreamsPageQuery::default()
        };
        let more = StreamsPage {
            streams: Vec::new(),
            next_after_id: Some(7),
        };
        assert_eq!(
            query.next(&more),
            Some(StreamsPageQuery {
                after_id: Some(7),
                ..query.clone()
            })
        );
        let last = StreamsPage {
            streams: Vec::new(),
            next_after_id: None,
        };
        assert_eq!(query.next(&last), None);
    }
}
