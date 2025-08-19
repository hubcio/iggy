#!/bin/bash
# Licensed to the Apache Software Foundation (ASF) under one
# or more contributor license agreements.  See the NOTICE file
# distributed with this work for additional information
# regarding copyright ownership.  The ASF licenses this file
# to you under the Apache License, Version 2.0 (the
# "License"); you may not use this file except in compliance
# with the License.  You may obtain a copy of the License at
#
#   http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing,
# software distributed under the License is distributed on an
# "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
# KIND, either express or implied.  See the License for the
# specific language governing permissions and limitations
# under the License.

set -e

SDK=${1:-"all"}
FEATURE=${2:-"scenarios/basic_messaging.feature"}

# Extract Rust version from rust-toolchain.toml
RUST_VERSION=$(sed -En 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' rust-toolchain.toml 2>/dev/null || echo "1.89")
export RUST_VERSION

# Enable Docker BuildKit for better caching
export DOCKER_BUILDKIT=1
export COMPOSE_DOCKER_CLI_BUILD=1

echo "🧪 Running BDD tests for SDK: $SDK"
echo "📁 Feature file: $FEATURE"
echo "🦀 Using Rust version: $RUST_VERSION"

# Save original directory
REPO_ROOT="$(dirname "$0")/.."

# If IGGY_SERVER_PATH is not set, use default
if [ -z "$IGGY_SERVER_PATH" ]; then
  IGGY_SERVER_PATH="target/debug/iggy-server"
  echo "ℹ️  IGGY_SERVER_PATH not set, using default: $IGGY_SERVER_PATH"
else
  echo "ℹ️  Using IGGY_SERVER_PATH: $IGGY_SERVER_PATH"
fi

# Verify server binary exists (from repo root)
if [ ! -f "$REPO_ROOT/$IGGY_SERVER_PATH" ]; then
  echo "❌ Error: Server binary not found at $REPO_ROOT/$IGGY_SERVER_PATH"
  echo "Current directory: $(pwd)"
  echo "Available files in target/debug:"
  find "$REPO_ROOT/target/debug/" -maxdepth 1 -type f -name "iggy*" 2>/dev/null | head -20 || echo "Directory not found"
  exit 1
fi

echo "✅ Server binary found at: $REPO_ROOT/$IGGY_SERVER_PATH"

# Export for docker-compose (path should be relative to context which is repo root)
export IGGY_SERVER_PATH

# Change to BDD directory
cd "$(dirname "$0")/../bdd"

case $SDK in
"rust")
  echo "🦀 Running Rust BDD tests..."
  docker compose build iggy-server rust-bdd
  docker compose up --abort-on-container-exit rust-bdd
  ;;
"python")
  echo "🐍 Running Python BDD tests..."
  docker compose build iggy-server python-bdd
  docker compose up --abort-on-container-exit python-bdd
  ;;
"go")
  echo "🐹 Running Go BDD tests..."
  docker compose build iggy-server go-bdd
  docker compose up --abort-on-container-exit go-bdd
  ;;
"node")
  echo "🐢🚀 Running node BDD tests..."
  docker compose build iggy-server node-bdd
  docker compose up --abort-on-container-exit node-bdd
  ;;
"csharp")
  echo "🔷 Running csharp BDD tests..."
  docker compose build iggy-server csharp-bdd
  docker compose up --abort-on-container-exit csharp-bdd
  ;;
"all")
  echo "🚀 Running all SDK BDD tests..."
  echo "🏗️ Building all containers with caching..."
  docker compose build iggy-server rust-bdd python-bdd go-bdd node-bdd csharp-bdd
  echo "🦀 Starting with Rust tests..."
  docker compose up --abort-on-container-exit rust-bdd
  echo "🐍 Now running Python tests..."
  docker compose up --abort-on-container-exit python-bdd
  echo "🐹 Now running Go tests..."
  docker compose up --abort-on-container-exit go-bdd
  echo "🐢🚀 Now running node BDD tests..."
  docker compose up --abort-on-container-exit node-bdd
  echo "🔷 Now running csharp BDD tests..."
  docker compose up --abort-on-container-exit csharp-bdd
  ;;
"clean")
  echo "🧹 Cleaning up Docker resources..."
  docker compose down -v
  docker compose rm -f
  ;;
*)
  echo "❌ Unknown SDK: $SDK"
  echo "📖 Usage: $0 [rust|python|go|all|clean] [feature_file]"
  echo "📖 Examples:"
  echo "   $0 rust                    # Run Rust tests only"
  echo "   $0 python                  # Run Python tests only"
  echo "   $0 go                      # Run Go tests only"
  echo "   $0 node                    # Run Node.js tests only"
  echo "   $0 csharp                  # Run csharp tests only"
  echo "   $0 all                     # Run all SDK tests"
  echo "   $0 clean                   # Clean up Docker resources"
  exit 1
  ;;
esac

echo "✅ BDD tests completed for: $SDK"
