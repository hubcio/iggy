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

import { invalidate } from '$app/navigation';
import { clientApi } from '#lib/api/clientApi.js';
import {
  STREAM_PAGE_LIMIT,
  readStreams,
  type StreamListView,
  type StreamPage
} from '#lib/api/streamPages.js';
import { streamMapper } from '#lib/domain/Stream.js';
import type { LayoutLoad } from './$types';

const STREAM_LIST_DEPENDENCY = 'app:stream-list';

// Kept here, not in the component, because the periodic refresh reruns `load`
// and must read the same streams the list shows. Module state is safe because
// the app renders only in the browser (`ssr = false`). SvelteKit allows other
// exports from a load module only with a `_` prefix.
export const _streamList: StreamListView = { name: '', count: STREAM_PAGE_LIMIT };

export function _showStreams(name: string, count: number): Promise<void> {
  _streamList.name = name;
  _streamList.count = count;
  return invalidate(STREAM_LIST_DEPENDENCY);
}

export const load: LayoutLoad = async ({ depends }) => {
  depends(STREAM_LIST_DEPENDENCY);
  const page = await readStreams(_streamList, (queryParams) =>
    clientApi<StreamPage>({ method: 'GET', path: '/stream-pages', queryParams })
  );

  return {
    streams: page.streams.map(streamMapper),
    nextAfterId: page.next_after_id
  };
};
