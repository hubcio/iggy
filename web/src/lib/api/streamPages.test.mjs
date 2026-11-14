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

import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  MAX_STREAM_NAME_BYTES,
  MAX_STREAM_PAGE_LIMIT,
  STREAM_PAGE_LIMIT,
  readStreams
} from './streamPages.ts';

/** Answers with `pages` in order and records each query it receives. */
function serve(pages) {
  const queries = [];
  const readPage = async (query) => {
    queries.push(query);
    return pages.shift() ?? assert.fail('unexpected page request');
  };
  return { queries, readPage };
}

test('given_no_search_when_reading_should_ask_for_one_page_without_a_name', async () => {
  const { queries, readPage } = serve([{ streams: [{ id: 1 }], next_after_id: null }]);
  const page = await readStreams({ name: '', count: STREAM_PAGE_LIMIT }, readPage);
  assert.deepEqual(queries, [{ limit: `${STREAM_PAGE_LIMIT}` }]);
  assert.deepEqual(page, { streams: [{ id: 1 }], next_after_id: null });
});

test('given_short_and_empty_pages_when_more_follow_should_keep_reading_after_each_cursor', async () => {
  const { queries, readPage } = serve([
    { streams: [{ id: 3 }], next_after_id: 65535 },
    { streams: [], next_after_id: 131071 },
    { streams: [{ id: 131080 }], next_after_id: null }
  ]);
  const page = await readStreams({ name: 'ord', count: 5 }, readPage);
  assert.deepEqual(queries, [
    { limit: '5', name: 'ord' },
    { after_id: '65535', limit: '4', name: 'ord' },
    { after_id: '131071', limit: '4', name: 'ord' }
  ]);
  assert.deepEqual(page, { streams: [{ id: 3 }, { id: 131080 }], next_after_id: null });
});

test('given_enough_streams_when_more_follow_should_stop_and_keep_the_cursor', async () => {
  const { queries, readPage } = serve([{ streams: [{ id: 0 }, { id: 4 }], next_after_id: 4 }]);
  const page = await readStreams({ name: '', count: 2 }, readPage);
  assert.equal(queries.length, 1);
  assert.deepEqual(page, { streams: [{ id: 0 }, { id: 4 }], next_after_id: 4 });
});

test('given_more_streams_than_one_request_allows_when_reading_should_cap_each_request', async () => {
  const first = Array.from({ length: MAX_STREAM_PAGE_LIMIT }, (_, id) => ({ id }));
  const { queries, readPage } = serve([
    { streams: first, next_after_id: MAX_STREAM_PAGE_LIMIT - 1 },
    { streams: [{ id: MAX_STREAM_PAGE_LIMIT }], next_after_id: null }
  ]);
  const page = await readStreams({ name: '', count: MAX_STREAM_PAGE_LIMIT + 1 }, readPage);
  assert.deepEqual(queries, [
    { limit: `${MAX_STREAM_PAGE_LIMIT}` },
    { after_id: `${MAX_STREAM_PAGE_LIMIT - 1}`, limit: '1' }
  ]);
  assert.equal(page.streams.length, MAX_STREAM_PAGE_LIMIT + 1);
  assert.equal(page.next_after_id, null);
});

test('given_a_search_longer_than_any_stream_name_when_reading_should_answer_empty_without_asking', async () => {
  // Two bytes per character: fewer characters than the limit, but more bytes.
  const tooLong = 'é'.repeat(Math.ceil((MAX_STREAM_NAME_BYTES + 1) / 2));
  const refused = serve([]);
  const page = await readStreams({ name: tooLong, count: STREAM_PAGE_LIMIT }, refused.readPage);
  assert.deepEqual(page, { streams: [], next_after_id: null });
  assert.equal(refused.queries.length, 0);

  const longest = 'n'.repeat(MAX_STREAM_NAME_BYTES);
  const accepted = serve([{ streams: [], next_after_id: null }]);
  await readStreams({ name: longest, count: STREAM_PAGE_LIMIT }, accepted.readPage);
  assert.equal(accepted.queries[0].name, longest);
});

test('given_a_cursor_that_does_not_advance_when_reading_should_fail_instead_of_looping', async () => {
  const { readPage } = serve([
    { streams: [], next_after_id: 7 },
    { streams: [], next_after_id: 7 }
  ]);
  const reading = readStreams({ name: 'ord', count: STREAM_PAGE_LIMIT }, readPage);
  await assert.rejects(reading, /did not advance/);
});
