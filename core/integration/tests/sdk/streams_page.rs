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

use iggy::prelude::*;
use iggy_binary_protocol::WireEncode;
use iggy_binary_protocol::codes::GET_STREAMS_PAGE_CODE;
use iggy_binary_protocol::requests::streams::GetStreamsPageRequest;
use iggy_common::{MAX_STREAMS_PAGE_LIMIT, StreamsPage, StreamsPageQuery};
use integration::iggy_harness;

const ORDERS_FILTER: &str = "orders";
const STREAM_NAMES: [&str; 7] = [
    "page-orders-0",
    "page-users-0",
    "page-orders-1",
    "page-users-1",
    "page-orders-2",
    "page-orders-3",
    "page-orders-4",
];
const DELETED_STREAM: &str = "page-orders-2";
const PAGE_LIMIT: u32 = 2;
const UNGRANTED_USERNAME: &str = "page-nobody";
const UNGRANTED_PASSWORD: &str = "page-nobody-pass";

#[iggy_harness(test_client_transport = [Tcp, Http, Quic, WebSocket])]
async fn given_streams_with_a_hole_when_walking_pages_should_list_each_stream_once_in_id_order(
    harness: &TestHarness,
) {
    let client = harness.root_client().await.expect("root client");
    for name in STREAM_NAMES {
        client
            .create_stream(name, &StreamCreateOptions::default())
            .await
            .expect("create stream");
    }
    client
        .delete_stream(&Identifier::named(DELETED_STREAM).expect("stream id"))
        .await
        .expect("delete stream");
    let listed = stream_ids(&client.get_streams().await.expect("get streams"));
    assert_eq!(listed.len(), STREAM_NAMES.len() - 1);

    let (walked, pages) = walk(
        &client,
        StreamsPageQuery {
            limit: PAGE_LIMIT,
            ..StreamsPageQuery::default()
        },
    )
    .await;
    assert_eq!(stream_ids(&walked), listed);
    assert_eq!(
        pages,
        listed.len().div_ceil(PAGE_LIMIT as usize),
        "a full last page must report the end instead of asking for an empty page"
    );

    let (orders, _) = walk(
        &client,
        StreamsPageQuery {
            limit: PAGE_LIMIT,
            name: Some(ORDERS_FILTER.to_owned()),
            ..StreamsPageQuery::default()
        },
    )
    .await;
    let mut expected: Vec<&str> = STREAM_NAMES
        .into_iter()
        .filter(|name| name.contains(ORDERS_FILTER) && *name != DELETED_STREAM)
        .collect();
    expected.sort_unstable();
    let mut names: Vec<&str> = orders.iter().map(|stream| stream.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, expected);

    let whole = client
        .get_streams_page(&StreamsPageQuery {
            limit: MAX_STREAMS_PAGE_LIMIT,
            ..StreamsPageQuery::default()
        })
        .await
        .expect("one page");
    assert_eq!(stream_ids(&whole.streams), listed);
    assert_eq!(whole.next_after_id, None);
}

#[iggy_harness(test_client_transport = [Tcp, Http, Quic, WebSocket])]
async fn given_a_limit_outside_the_page_range_when_paging_should_be_refused_by_sdk_and_server(
    harness: &TestHarness,
) {
    let client = harness.root_client().await.expect("root client");
    let http = matches!(
        client.get_connection_info().await.protocol,
        TransportProtocol::Http
    );
    for limit in [0, MAX_STREAMS_PAGE_LIMIT + 1] {
        let refused = client
            .get_streams_page(&StreamsPageQuery {
                limit,
                ..StreamsPageQuery::default()
            })
            .await
            .expect_err("the SDK refuses the limit");
        assert_eq!(refused.as_code(), IggyError::InvalidCommand.as_code());
        if http {
            assert_http_refused(&client, &format!("limit={limit}")).await;
        } else {
            let request = GetStreamsPageRequest {
                after_id: None,
                limit,
                name: None,
            };
            assert_binary_refused(&client, request.to_bytes().to_vec()).await;
        }
    }
    if http {
        for query in [
            "limit=ten",
            "limit=010",
            "after_id=-1",
            "name=",
            "limit=5&limit=5",
            "unknown=1",
        ] {
            assert_http_refused(&client, query).await;
        }
    } else {
        let mut trailing = GetStreamsPageRequest {
            after_id: None,
            limit: PAGE_LIMIT,
            name: None,
        }
        .to_bytes()
        .to_vec();
        trailing.push(0);
        assert_binary_refused(&client, trailing).await;
    }
}

#[iggy_harness(test_client_transport = [Tcp, Http, Quic, WebSocket])]
async fn given_a_user_without_stream_permissions_when_paging_should_be_unauthorized(
    harness: &TestHarness,
) {
    let root = harness.root_client().await.expect("root client");
    root.create_user(
        UNGRANTED_USERNAME,
        UNGRANTED_PASSWORD,
        UserStatus::Active,
        None,
        &UserCreateOptions::default(),
    )
    .await
    .expect("create user");
    let nobody = harness.new_client().await.expect("client");
    nobody
        .login_user(UNGRANTED_USERNAME, UNGRANTED_PASSWORD)
        .await
        .expect("login");
    let refused = nobody
        .get_streams_page(&StreamsPageQuery::default())
        .await
        .expect_err("an ungranted user cannot list streams");
    assert_eq!(refused.as_code(), IggyError::Unauthorized.as_code());
}

/// Walk every page of `query` and return the streams and the page count.
async fn walk(client: &IggyClient, mut query: StreamsPageQuery) -> (Vec<Stream>, usize) {
    let mut streams = Vec::new();
    let mut pages = 0;
    loop {
        let page: StreamsPage = client.get_streams_page(&query).await.expect("page");
        pages += 1;
        let next = query.next(&page);
        streams.extend(page.streams);
        let Some(next) = next else {
            return (streams, pages);
        };
        query = next;
    }
}

fn stream_ids(streams: &[Stream]) -> Vec<u32> {
    streams.iter().map(|stream| stream.id).collect()
}

async fn assert_binary_refused(client: &IggyClient, body: Vec<u8>) {
    let error = client
        .send_binary_request(GET_STREAMS_PAGE_CODE, body.into())
        .await
        .expect_err("the server refuses the request");
    assert_eq!(
        error.as_code(),
        IggyError::InvalidCommand.as_code(),
        "{error:?}"
    );
}

async fn assert_http_refused(client: &IggyClient, query: &str) {
    let error = client
        .send_http_request(HttpMethod::Get, &format!("/stream-pages?{query}"), None)
        .await
        .expect_err(query);
    assert!(
        matches!(error, IggyError::HttpResponseError(400, _)),
        "{query}: {error:?}"
    );
}
