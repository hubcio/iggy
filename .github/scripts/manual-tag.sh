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

# Configuration from environment
TARGET_COMPONENT="${TARGET_COMPONENT:-all}"
TARGET_VERSION="${TARGET_VERSION}"
TARGET_COMMIT="${TARGET_COMMIT}"
USE_V_PREFIX="${USE_V_PREFIX:-false}"
DRY_RUN="${DRY_RUN:-true}"

# Counters
TAGS_CREATED=0
TAGS_SKIPPED=0
TAGS_FAILED=0

# Color codes for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Component definitions
declare -A COMPONENTS=(
    ["server"]="core/server/Cargo.toml|server-|Apache Iggy Server"
    ["cli"]="core/cli/Cargo.toml|cli-|Apache Iggy CLI"
    ["sdk"]="core/sdk/Cargo.toml|iggy-|Apache Iggy SDK"
    ["bench-dashboard"]="core/bench/dashboard/server/Cargo.toml|bench-dashboard-|Apache Iggy Bench Dashboard"
)

# Function to check if tag exists
tag_exists() {
    git tag -l "$1" | grep -q "^$1$"
}

# Function to find commit where version was introduced
find_version_commit() {
    local cargo_path=$1
    local version=$2
    
    echo -e "  ${CYAN}🔍 Searching for version $version in $cargo_path history...${NC}" >&2
    
    # Find the first commit where this version appeared
    local commit=$(git log --reverse --format="%H" -p -- "$cargo_path" | \
        awk -v ver="version = \"$version\"" '
            /^commit / {commit=$2}
            $0 ~ ver && !found {print commit; found=1; exit}
        ')
    
    if [ -z "$commit" ]; then
        echo -e "  ${YELLOW}⚠️  Version $version not found in $cargo_path history${NC}" >&2
        return 1
    fi
    
    # Get commit info for display
    local commit_info=$(git log -1 --format="%h - %s (%ai)" "$commit")
    echo -e "  ${GREEN}✓ Found at commit: $commit_info${NC}" >&2
    
    echo "$commit"
}

# Function to validate commit exists
validate_commit() {
    local commit=$1
    
    if git rev-parse --verify "$commit^{commit}" >/dev/null 2>&1; then
        return 0
    else
        echo -e "${RED}❌ Error: Commit $commit does not exist${NC}"
        return 1
    fi
}

# Function to create tag
create_tag() {
    local tag_name=$1
    local component_name=$2
    local version=$3
    local commit_hash=$4
    local cargo_path=$5
    
    # Get commit details for the tag message
    local commit_date=$(git log -1 --format="%ai" "$commit_hash")
    local commit_author=$(git log -1 --format="%an <%ae>" "$commit_hash")
    local commit_subject=$(git log -1 --format="%s" "$commit_hash")
    
    if [ "$DRY_RUN" = "true" ]; then
        echo -e "${YELLOW}[DRY RUN]${NC} Would create tag: ${BLUE}$tag_name${NC}"
        echo -e "  ${CYAN}→ Component:${NC} $component_name"
        echo -e "  ${CYAN}→ Version:${NC} $version"
        echo -e "  ${CYAN}→ Commit:${NC} ${commit_hash:0:8} - $commit_subject"
        echo -e "  ${CYAN}→ Date:${NC} $commit_date"
        TAGS_CREATED=$((TAGS_CREATED + 1))
        return 0
    fi
    
    # Create annotated tag with detailed message
    local tag_message="Release: $component_name v$version

This tag was manually created for retroactive release.

Component: $component_name
Version: $version
Cargo.toml: $cargo_path
Original commit: $commit_hash
Original date: $commit_date
Original author: $commit_author

Commit message: $commit_subject

Tagged by: GitHub Actions (manual trigger)
Tagged on: $(date -u +"%Y-%m-%d %H:%M:%S UTC")"
    
    if git tag -a "$tag_name" "$commit_hash" -m "$tag_message"; then
        # Push the tag
        if git push origin "$tag_name"; then
            echo -e "${GREEN}✅ Created and pushed tag:${NC} ${BLUE}$tag_name${NC}"
            echo -e "  ${CYAN}→ Component:${NC} $component_name"
            echo -e "  ${CYAN}→ Version:${NC} $version"
            echo -e "  ${CYAN}→ Commit:${NC} ${commit_hash:0:8} - $commit_subject"
            TAGS_CREATED=$((TAGS_CREATED + 1))
            return 0
        else
            echo -e "${RED}❌ Failed to push tag:${NC} $tag_name"
            # Clean up local tag if push failed
            git tag -d "$tag_name" >/dev/null 2>&1
            TAGS_FAILED=$((TAGS_FAILED + 1))
            return 1
        fi
    else
        echo -e "${RED}❌ Failed to create tag:${NC} $tag_name"
        TAGS_FAILED=$((TAGS_FAILED + 1))
        return 1
    fi
}

# Function to process a single component
process_component() {
    local component=$1
    local component_info="${COMPONENTS[$component]}"
    
    if [ -z "$component_info" ]; then
        echo -e "${RED}❌ Unknown component: $component${NC}"
        return 1
    fi
    
    IFS='|' read -r cargo_path tag_prefix display_name <<< "$component_info"
    
    echo -e "\n${BLUE}Processing $display_name${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    
    # Determine tag name
    if [ "$USE_V_PREFIX" = "true" ]; then
        tag_name="${tag_prefix}v${TARGET_VERSION}"
    else
        tag_name="${tag_prefix}${TARGET_VERSION}"
    fi
    
    echo -e "Tag name: ${BLUE}$tag_name${NC}"
    
    # Check if tag already exists
    if tag_exists "$tag_name"; then
        echo -e "${YELLOW}⏭️  Tag already exists, skipping${NC}"
        TAGS_SKIPPED=$((TAGS_SKIPPED + 1))
        return 0
    fi
    
    # Determine commit to tag
    local commit_to_tag=""
    
    if [ -n "$TARGET_COMMIT" ]; then
        echo -e "Using specified commit: ${CYAN}$TARGET_COMMIT${NC}"
        if validate_commit "$TARGET_COMMIT"; then
            commit_to_tag="$TARGET_COMMIT"
        else
            return 1
        fi
    else
        echo -e "Auto-detecting commit for version ${CYAN}$TARGET_VERSION${NC}..."
        commit_to_tag=$(find_version_commit "$cargo_path" "$TARGET_VERSION")
        if [ -z "$commit_to_tag" ]; then
            echo -e "${YELLOW}⚠️  Could not find version $TARGET_VERSION in $cargo_path history${NC}"
            echo -e "${YELLOW}   Skipping $display_name${NC}"
            return 0
        fi
    fi
    
    # Create the tag
    create_tag "$tag_name" "$display_name" "$TARGET_VERSION" "$commit_to_tag" "$cargo_path"
}

# Main function
main() {
    echo -e "${BLUE}🏷️  Manual Component Tagging${NC}"
    echo "════════════════════════════════════════"
    
    # Display configuration
    echo -e "\n${CYAN}Configuration:${NC}"
    echo -e "  Component(s): ${BLUE}$TARGET_COMPONENT${NC}"
    echo -e "  Version: ${BLUE}$TARGET_VERSION${NC}"
    echo -e "  Commit: ${BLUE}${TARGET_COMMIT:-auto-detect}${NC}"
    echo -e "  V-prefix: ${BLUE}$USE_V_PREFIX${NC}"
    echo -e "  Mode: ${YELLOW}${DRY_RUN:+DRY RUN}${NC}${GREEN}${DRY_RUN:+|PRODUCTION}${NC}"
    
    # Validate required inputs
    if [ -z "$TARGET_VERSION" ]; then
        echo -e "\n${RED}❌ Error: TARGET_VERSION is required${NC}"
        exit 1
    fi
    
    # Process components
    if [ "$TARGET_COMPONENT" = "all" ]; then
        echo -e "\n${CYAN}Processing all components...${NC}"
        for component in "${!COMPONENTS[@]}"; do
            process_component "$component"
        done
    else
        process_component "$TARGET_COMPONENT"
    fi
    
    # Summary
    echo -e "\n════════════════════════════════════════"
    echo -e "${BLUE}📊 Summary:${NC}"
    echo -e "  Tags created: ${GREEN}$TAGS_CREATED${NC}"
    echo -e "  Tags skipped: ${YELLOW}$TAGS_SKIPPED${NC}"
    echo -e "  Tags failed: ${RED}$TAGS_FAILED${NC}"
    
    # Special output for GitHub Actions
    echo "TAGS_CREATED: $TAGS_CREATED"
    
    if [ "$DRY_RUN" = "true" ]; then
        echo -e "\n${YELLOW}🧪 This was a DRY RUN. No tags were actually created.${NC}"
        echo -e "${YELLOW}   To create tags, run with dry_run=false${NC}"
    elif [ $TAGS_CREATED -gt 0 ]; then
        echo -e "\n${GREEN}✅ Successfully created $TAGS_CREATED tag(s)!${NC}"
        echo -e "${GREEN}   The publish workflows will be triggered automatically.${NC}"
    elif [ $TAGS_FAILED -gt 0 ]; then
        echo -e "\n${RED}❌ Some tags failed to create. Please check the errors above.${NC}"
        exit 1
    else
        echo -e "\n${YELLOW}ℹ️  No new tags were created (all already exist or not found).${NC}"
    fi
}

# Run main function
main "$@"