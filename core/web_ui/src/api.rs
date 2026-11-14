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

use gloo_net::http::{Request, RequestBuilder, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::auth::Session;
use crate::models::{
    IdentityInfo, LoginUser, PolledMessages, StreamDetails, StreamsPage, TopicDetails,
    UserInfoDetails,
};

/// The most streams one page of the streams list holds.
pub const STREAMS_PAGE_SIZE: u32 = 100;

/// Empty by default so requests go to the server that served the UI. Set at
/// build time to point `dx serve` at a separately running server, see the
/// crate README.
const API_BASE_URL: &str = match option_env!("IGGY_WEB_UI_API_URL") {
    Some(url) => url,
    None => "",
};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ApiError {
    /// The session is not valid any more.
    #[error("session expired or not authorized")]
    Unauthorized,
    /// The account lacks a permission. The session stays valid.
    #[error("you do not have permission to do this")]
    Forbidden,
    #[error("not found")]
    NotFound,
    /// The server rejected the request and said why.
    #[error("{0}")]
    Rejected(String),
    #[error("server responded with {status}: {body}")]
    Status { status: u16, body: String },
    #[error("request failed: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

/// Thin client for the server's HTTP API, scoped to one session.
#[derive(Debug, Clone, Default)]
pub struct ApiClient {
    access_token: Option<String>,
}

impl ApiClient {
    pub fn new(session: Option<&Session>) -> Self {
        Self {
            access_token: session.map(|session| session.access_token.clone()),
        }
    }

    pub async fn login(&self, credentials: &LoginUser) -> Result<IdentityInfo, ApiError> {
        let request = self
            .builder(Request::post(&url("/users/login")))
            .json(credentials)
            .map_err(|error| ApiError::Network(error.to_string()))?;
        let response = request
            .send()
            .await
            .map_err(|error| ApiError::Network(error.to_string()))?;
        decode(response).await
    }

    pub async fn logout(&self) -> Result<(), ApiError> {
        let response = self
            .builder(Request::delete(&url("/users/logout")))
            .send()
            .await
            .map_err(|error| ApiError::Network(error.to_string()))?;
        check_status(response).await.map(drop)
    }

    pub async fn user(&self, user_id: u32) -> Result<UserInfoDetails, ApiError> {
        self.get(&format!("/users/{user_id}"), &[]).await
    }

    /// Streams in ascending id after `after_id`, up to `limit`. An empty `name`
    /// reads every stream, a non-empty one only the streams whose name contains
    /// it, case-sensitive.
    pub async fn streams_page(
        &self,
        after_id: Option<u32>,
        limit: u32,
        name: &str,
    ) -> Result<StreamsPage, ApiError> {
        self.get("/stream-pages", &streams_page_query(after_id, limit, name))
            .await
    }

    pub async fn stream(&self, stream_id: u32) -> Result<StreamDetails, ApiError> {
        self.get(&format!("/streams/{stream_id}"), &[]).await
    }

    pub async fn topic(&self, stream_id: u32, topic_id: u32) -> Result<TopicDetails, ApiError> {
        self.get(&format!("/streams/{stream_id}/topics/{topic_id}"), &[])
            .await
    }

    /// Reads without committing, so browsing never moves a consumer offset.
    pub async fn poll_messages(
        &self,
        stream_id: u32,
        topic_id: u32,
        partition_id: u32,
        offset: u64,
        count: u32,
    ) -> Result<PolledMessages, ApiError> {
        let query = [
            ("kind", "offset".to_owned()),
            ("value", offset.to_string()),
            ("count", count.to_string()),
            ("auto_commit", "false".to_owned()),
            ("partition_id", partition_id.to_string()),
        ];
        self.get(
            &format!("/streams/{stream_id}/topics/{topic_id}/messages"),
            &query,
        )
        .await
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ApiError> {
        let builder =
            Request::get(&url(path)).query(query.iter().map(|(key, value)| (*key, value.as_str())));
        let response = self
            .builder(builder)
            .send()
            .await
            .map_err(|error| ApiError::Network(error.to_string()))?;
        decode(response).await
    }

    fn builder(&self, builder: RequestBuilder) -> RequestBuilder {
        match &self.access_token {
            Some(token) => builder.header("Authorization", &format!("Bearer {token}")),
            None => builder,
        }
    }
}

fn url(path: &str) -> String {
    format!("{API_BASE_URL}{path}")
}

/// The server refuses an empty `name`, so an empty search leaves it out.
fn streams_page_query(
    after_id: Option<u32>,
    limit: u32,
    name: &str,
) -> Vec<(&'static str, String)> {
    [
        after_id.map(|after_id| ("after_id", after_id.to_string())),
        Some(("limit", limit.to_string())),
        (!name.is_empty()).then(|| ("name", name.to_owned())),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Reads the body as bytes, because `Response::json` goes through one JS
/// string, which browsers cap well below a page of large messages.
async fn decode<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
    let body = check_status(response)
        .await?
        .binary()
        .await
        .map_err(|error| ApiError::Network(error.to_string()))?;
    serde_json::from_slice(&body).map_err(|error| ApiError::Decode(error.to_string()))
}

async fn check_status(response: Response) -> Result<Response, ApiError> {
    match response.status() {
        200..=299 => Ok(response),
        401 => Err(ApiError::Unauthorized),
        403 => Err(ApiError::Forbidden),
        404 => Err(ApiError::NotFound),
        status => {
            let body = response.text().await.unwrap_or_default();
            Err(match serde_json::from_str::<ErrorBody>(&body) {
                Ok(error) => ApiError::Rejected(error.reason),
                Err(_) => ApiError::Status { status, body },
            })
        }
    }
}

/// The JSON body the server attaches to rejected requests.
#[derive(Deserialize)]
struct ErrorBody {
    reason: String,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use iggy_common::StreamsPageQuery;

    use super::*;

    #[test]
    fn given_page_arguments_should_build_the_query_the_server_parses() {
        // The server refuses a limit out of its range, an unknown key and an
        // empty name, so a parsed query proves the page size, the key names
        // and that an empty search sends no name.
        let cases = [
            (
                (None, STREAMS_PAGE_SIZE, ""),
                StreamsPageQuery {
                    after_id: None,
                    limit: STREAMS_PAGE_SIZE,
                    name: None,
                },
            ),
            (
                (Some(41), STREAMS_PAGE_SIZE, "orders eu/1"),
                StreamsPageQuery {
                    after_id: Some(41),
                    limit: STREAMS_PAGE_SIZE,
                    name: Some("orders eu/1".to_owned()),
                },
            ),
        ];
        for ((after_id, limit, name), expected) in cases {
            let query = streams_page_query(after_id, limit, name);
            let parsed = StreamsPageQuery::from_http_query(
                query.iter().map(|(key, value)| (*key, value.as_str())),
            );
            assert_eq!(parsed, Ok(expected), "{query:?}");
        }
    }
}
