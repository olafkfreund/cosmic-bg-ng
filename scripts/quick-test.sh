#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Quick verification script for cosmic-bg-ng
# Runs cargo tests and basic runtime checks

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  cosmic-bg-ng Quick Test${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo ""

cd "$PROJECT_DIR"

#------------------------------------------------------------------------------
# 1. Unit Tests
#------------------------------------------------------------------------------
echo -e "${YELLOW}[1/5] Running unit tests...${NC}"
if cargo test --quiet 2>&1; then
    echo -e "${GREEN}✓ Unit tests passed${NC}"
else
    echo -e "${RED}✗ Unit tests failed${NC}"
    exit 1
fi

#------------------------------------------------------------------------------
# 2. Check Build
#------------------------------------------------------------------------------
echo -e "${YELLOW}[2/5] Checking release build...${NC}"
if cargo build --release --quiet 2>&1; then
    echo -e "${GREEN}✓ Release build successful${NC}"
else
    echo -e "${RED}✗ Release build failed${NC}"
    exit 1
fi

#------------------------------------------------------------------------------
# 3. Verify Binary Features
#------------------------------------------------------------------------------
echo -e "${YELLOW}[3/5] Verifying compiled features...${NC}"

BINARY="target/release/cosmic-bg"

if [ -f "$BINARY" ]; then
    # Check linked libraries
    echo "  Checking linked libraries..."

    if ldd "$BINARY" 2>/dev/null | grep -q "libgstreamer"; then
        echo -e "  ${GREEN}✓ GStreamer (video support)${NC}"
    else
        echo -e "  ${YELLOW}○ GStreamer not linked (video support may be limited)${NC}"
    fi

    if ldd "$BINARY" 2>/dev/null | grep -q "libvulkan\|libwgpu"; then
        echo -e "  ${GREEN}✓ GPU libraries detected${NC}"
    else
        echo -e "  ${YELLOW}○ GPU libraries may be loaded at runtime${NC}"
    fi

    echo -e "${GREEN}✓ Binary verified${NC}"
else
    echo -e "${RED}✗ Binary not found${NC}"
    exit 1
fi

#------------------------------------------------------------------------------
# 4. Runtime Check
#------------------------------------------------------------------------------
echo -e "${YELLOW}[4/5] Checking runtime environment...${NC}"

# Check Wayland
if [ -n "${WAYLAND_DISPLAY:-}" ]; then
    echo -e "  ${GREEN}✓ Wayland display: $WAYLAND_DISPLAY${NC}"
else
    echo -e "  ${YELLOW}○ No Wayland display (run within COSMIC session)${NC}"
fi

# Check if cosmic-bg is running
if pgrep -x "cosmic-bg" > /dev/null 2>&1; then
    PID=$(pgrep -x "cosmic-bg")
    echo -e "  ${GREEN}✓ cosmic-bg running (PID: $PID)${NC}"
else
    echo -e "  ${YELLOW}○ cosmic-bg not running${NC}"
fi

# Check GStreamer
if command -v gst-inspect-1.0 &> /dev/null; then
    GST_VERSION=$(gst-inspect-1.0 --version | head -1)
    echo -e "  ${GREEN}✓ $GST_VERSION${NC}"
else
    echo -e "  ${YELLOW}○ GStreamer CLI not found${NC}"
fi

# Check Vulkan
if command -v vulkaninfo &> /dev/null; then
    GPU_NAME=$(vulkaninfo --summary 2>/dev/null | grep "deviceName" | head -1 | sed 's/.*= //' || echo "Unknown")
    echo -e "  ${GREEN}✓ Vulkan GPU: $GPU_NAME${NC}"
else
    echo -e "  ${YELLOW}○ vulkaninfo not found${NC}"
fi

echo -e "${GREEN}✓ Runtime environment checked${NC}"

#------------------------------------------------------------------------------
# 5. Configuration Check
#------------------------------------------------------------------------------
echo -e "${YELLOW}[5/5] Checking configuration...${NC}"

CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/com.system76.CosmicBackground/v1"

if [ -d "$CONFIG_DIR" ]; then
    echo -e "  ${GREEN}✓ Config directory exists: $CONFIG_DIR${NC}"

    if [ -f "$CONFIG_DIR/all" ]; then
        echo "  Current wallpaper config:"
        head -3 "$CONFIG_DIR/all" | sed 's/^/    /'
    fi
else
    echo -e "  ${YELLOW}○ Config directory not found (will be created on first run)${NC}"
fi

echo -e "${GREEN}✓ Configuration checked${NC}"

#------------------------------------------------------------------------------
# Summary
#------------------------------------------------------------------------------
echo ""
echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
echo -e "${GREEN}All quick tests passed!${NC}"
echo ""
echo "For comprehensive feature testing, run:"
echo "  ./scripts/test-features.sh --all"
echo ""
echo -e "${BLUE}═══════════════════════════════════════════════════════════${NC}"
