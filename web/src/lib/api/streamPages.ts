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

/** Streams a list loads at first and adds on each "Load more". */
export const STREAM_PAGE_LIMIT = 100;
/** The most streams one `GET /stream-pages` request may ask for. */
export const MAX_STREAM_PAGE_LIMIT = 1000;
/** The longest stream name in UTF-8 bytes. The server refuses a longer `name` filter. */
export const MAX_STREAM_NAME_BYTES = 255;

/** The reply of `GET /stream-pages`, streams in ascending id. */
export type StreamPage = {
  streams: unknown[];
  next_after_id: number | null;
};

/** The first `count` streams whose name contains `name`. An empty name matches every stream. */
export type StreamListView = {
  name: string;
  count: number;
};

/**
 * Reads pages until the list holds `count` streams or the server sends the last
 * page. A page can hold fewer streams than asked, even none, while more follow,
 * so only `next_after_id: null` ends the walk early.
 */
export async function readStreams(
  { name, count }: StreamListView,
  readPage: (query: Record<string, string>) => Promise<StreamPage>
): Promise<StreamPage> {
  // No stream name can contain a longer text, and the server refuses it.
  if (new TextEncoder().encode(name).length > MAX_STREAM_NAME_BYTES) {
    return { streams: [], next_after_id: null };
  }
  const streams: unknown[] = [];
  let afterId: number | undefined;
  for (;;) {
    const limit = Math.min(count - streams.length, MAX_STREAM_PAGE_LIMIT);
    const page = await readPage(streamPageQuery(name, limit, afterId));
    streams.push(...page.streams);
    if (page.next_after_id === null || streams.length >= count) {
      return { streams, next_after_id: page.next_after_id };
    }
    if (afterId !== undefined && page.next_after_id <= afterId) {
      throw new Error('The server did not advance the stream page cursor.');
    }
    afterId = page.next_after_id;
  }
}

function streamPageQuery(name: string, limit: number, afterId?: number): Record<string, string> {
  const query: Record<string, string> = { limit: `${limit}` };
  if (afterId !== undefined) {
    query.after_id = `${afterId}`;
  }
  // An empty filter matches every stream, and the server refuses it.
  if (name !== '') {
    query.name = name;
  }
  return query;
}
