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

use dioxus::prelude::*;

use crate::Route;
use crate::api::{ApiClient, ApiError, STREAMS_PAGE_SIZE};
use crate::components::{Crumb, ErrorView, Loading, PageHeader, Stat, api_client, use_session};
use crate::format;
use crate::models::StreamsPage;

#[component]
pub fn Streams() -> Element {
    let session = use_session();
    let mut name = use_signal(String::new);
    // The `next_after_id` of each page before the shown one. The server only
    // reads forward, so Previous drops the last one and reads again.
    let mut cursors = use_signal(Vec::<u32>::new);
    let page = use_resource(move || async move {
        let after_id = cursors.read().last().copied();
        read_page(&api_client(session), after_id, &name()).await
    });

    let next_after_id = page
        .read()
        .as_ref()
        .and_then(|shown| shown.as_ref().ok()?.next_after_id);
    let previous_page = move |_| {
        cursors.write().pop();
    };
    // While a page loads, the page still shown is stale: its cursor would
    // repeat this step or continue the previous search.
    let next_page = move |_| {
        if let Some(after_id) = next_after_id
            && !page.pending()
        {
            cursors.write().push(after_id);
        }
    };

    let body = match &*page.read() {
        None => rsx! { Loading {} },
        Some(Err(error)) => rsx! { ErrorView { error: error.clone() } },
        Some(Ok(shown)) if shown.streams.is_empty() => rsx! {
            p { class: "muted",
                if !cursors.read().is_empty() {
                    "No more streams."
                } else if name.read().is_empty() {
                    "No streams yet."
                } else {
                    "No streams match."
                }
            }
        },
        Some(Ok(shown)) => rsx! {
            table {
                thead {
                    tr {
                        th { "ID" }
                        th { "Name" }
                        th { class: "num", "Topics" }
                        th { class: "num", "Messages" }
                        th { class: "num", "Size" }
                        th { "Created" }
                    }
                }
                tbody {
                    for stream in &shown.streams {
                        tr { key: "{stream.id}",
                            td { "{stream.id}" }
                            td {
                                Link { to: Route::StreamPage { stream_id: stream.id }, "{stream.name}" }
                            }
                            td { class: "num", "{stream.topics_count}" }
                            td { class: "num", "{stream.messages_count}" }
                            td { class: "num", "{stream.size}" }
                            td { {format::timestamp(stream.created_at)} }
                        }
                    }
                }
            }
        },
    };

    rsx! {
        PageHeader { title: "Streams" }
        div { class: "pager",
            input {
                r#type: "search",
                placeholder: "Search by name",
                value: name,
                oninput: move |event| {
                    name.set(event.value());
                    cursors.write().clear();
                },
            }
            button {
                class: "secondary",
                disabled: cursors.read().is_empty(),
                onclick: previous_page,
                "Previous"
            }
            button {
                class: "secondary",
                disabled: next_after_id.is_none(),
                onclick: next_page,
                "Next"
            }
        }
        {body}
    }
}

#[component]
pub fn StreamPage(stream_id: u32) -> Element {
    let session = use_session();
    let stream = use_resource(use_reactive!(|(stream_id,)| async move {
        api_client(session).stream(stream_id).await
    }));

    let body = match &*stream.read() {
        None => rsx! { Loading {} },
        Some(Err(error)) => rsx! { ErrorView { error: error.clone() } },
        Some(Ok(stream)) => rsx! {
            PageHeader { title: stream.name.clone(),
                Crumb { to: Route::Streams {}, label: "Streams" }
            }
            div { class: "stats",
                Stat { label: "ID", value: stream.id.to_string() }
                Stat { label: "Topics", value: stream.topics_count.to_string() }
                Stat { label: "Messages", value: stream.messages_count.to_string() }
                Stat { label: "Size", value: stream.size.clone() }
                Stat { label: "Created", value: format::timestamp(stream.created_at) }
            }
            h2 { "Topics" }
            if stream.topics.is_empty() {
                p { class: "muted", "No topics in this stream." }
            } else {
                table {
                    thead {
                        tr {
                            th { "ID" }
                            th { "Name" }
                            th { class: "num", "Partitions" }
                            th { class: "num", "Messages" }
                            th { class: "num", "Size" }
                            th { "Message expiry" }
                            th { "Created" }
                        }
                    }
                    tbody {
                        for topic in &stream.topics {
                            tr { key: "{topic.id}",
                                td { "{topic.id}" }
                                td {
                                    Link {
                                        to: Route::TopicPage { stream_id, topic_id: topic.id },
                                        "{topic.name}"
                                    }
                                }
                                td { class: "num", "{topic.partitions_count}" }
                                td { class: "num", "{topic.messages_count}" }
                                td { class: "num", "{topic.size}" }
                                td { {format::message_expiry(&topic.options)} }
                                td { {format::timestamp(topic.created_at)} }
                            }
                        }
                    }
                }
            }
        },
    };

    rsx! { {body} }
}

/// Reads on until the page holds `STREAMS_PAGE_SIZE` streams or no stream
/// follows. One request scans a bounded number of streams, so a rare name can
/// come back short, or empty, long before the last stream.
async fn read_page(
    client: &ApiClient,
    mut after_id: Option<u32>,
    name: &str,
) -> Result<StreamsPage, ApiError> {
    let mut streams = Vec::new();
    let mut remaining = STREAMS_PAGE_SIZE;
    loop {
        let page = client.streams_page(after_id, remaining, name).await?;
        remaining =
            u32::try_from(page.streams.len()).map_or(0, |found| remaining.saturating_sub(found));
        streams.extend(page.streams);
        after_id = page.next_after_id;
        if remaining == 0 || after_id.is_none() {
            return Ok(StreamsPage {
                streams,
                next_after_id: after_id,
            });
        }
    }
}
