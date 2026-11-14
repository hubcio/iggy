/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

#include <cstddef>
#include <cstdint>
#include <limits>
#include <optional>
#include <string>
#include <utility>

#include <gtest/gtest.h>

#include "iggy.hpp"

namespace {

constexpr std::uint32_t kInvalidCommand = 3;
constexpr std::size_t kMaxNameBytes     = 255;

}  // namespace

TEST(StreamsPageQueryTest, GivenDefaultQueryWhenReadingShouldAskForTheFirstPageWithTheDefaultLimit) {
    const iggy::StreamsPageQuery query;

    EXPECT_FALSE(query.AfterId().has_value());
    EXPECT_EQ(query.Limit(), iggy::StreamsPageQuery::kDefaultLimit);
    EXPECT_FALSE(query.Name().has_value());
    EXPECT_NO_THROW(query.Validate());
}

TEST(StreamsPageQueryTest, GivenValuesAtTheBoundsWhenValidatingShouldAcceptThem) {
    const std::pair<const char *, iggy::StreamsPageQuery> accepted[] = {
        {"smallest limit", iggy::StreamsPageQuery().SetLimit(1)},
        {"largest limit", iggy::StreamsPageQuery().SetLimit(iggy::StreamsPageQuery::kMaxLimit)},
        {"largest after ID", iggy::StreamsPageQuery().SetAfterId(std::numeric_limits<std::uint32_t>::max())},
        {"longest name", iggy::StreamsPageQuery().SetName(std::string(kMaxNameBytes, 'n'))},
        {"multi-byte name", iggy::StreamsPageQuery().SetName("żółw 🚀")},
    };
    for (const auto &[description, query] : accepted) {
        SCOPED_TRACE(description);
        EXPECT_NO_THROW(query.Validate());
    }
}

TEST(StreamsPageQueryTest, GivenRefusedValuesWhenValidatingShouldThrowInvalidCommand) {
    const std::pair<const char *, iggy::StreamsPageQuery> refused[] = {
        {"zero limit", iggy::StreamsPageQuery().SetLimit(0)},
        {"limit above the maximum", iggy::StreamsPageQuery().SetLimit(iggy::StreamsPageQuery::kMaxLimit + 1)},
        {"empty name", iggy::StreamsPageQuery().SetName("")},
        {"name longer than a stream name", iggy::StreamsPageQuery().SetName(std::string(kMaxNameBytes + 1, 'n'))},
        {"name that is not UTF-8", iggy::StreamsPageQuery().SetName("\xff")},
    };
    for (const auto &[description, query] : refused) {
        SCOPED_TRACE(description);
        try {
            query.Validate();
            ADD_FAILURE() << "an invalid query must be refused";
        } catch (const iggy::IggyException &error) {
            EXPECT_EQ(error.Code(), std::optional<std::uint32_t>(kInvalidCommand)) << error.what();
            EXPECT_EQ(error.Kind(), "invalid_command") << error.what();
        }
    }
}
