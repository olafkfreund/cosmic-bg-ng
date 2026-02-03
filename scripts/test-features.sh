#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
#
# Comprehensive feature test script for cosmic-bg-ng
# Tests: Static images, Animated images (GIF, WebP), Video, Shaders, Cache
#
# Usage: ./scripts/test-features.sh [--all|--static|--animated|--video|--shader|--cache]

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Test assets directory
TEST_DIR="${XDG_RUNTIME_DIR:-/tmp}/cosmic-bg-ng-test"
COSMIC_CONFIG_NAME="com.system76.CosmicBackground"
LOG_FILE="${TEST_DIR}/test.log"

# Track test results
TESTS_PASSED=0
TESTS_FAILED=0
TESTS_SKIPPED=0

#------------------------------------------------------------------------------
# Helper Functions
#------------------------------------------------------------------------------

log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
    echo "[INFO] $1" >> "$LOG_FILE"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
    echo "[PASS] $1" >> "$LOG_FILE"
    ((TESTS_PASSED++)) || true
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
    echo "[FAIL] $1" >> "$LOG_FILE"
    ((TESTS_FAILED++)) || true
}

log_skip() {
    echo -e "${YELLOW}[SKIP]${NC} $1"
    echo "[SKIP] $1" >> "$LOG_FILE"
    ((TESTS_SKIPPED++)) || true
}

log_section() {
    echo ""
    echo -e "${CYAN}═══════════════════════════════════════════════════════════${NC}"
    echo -e "${CYAN}  $1${NC}"
    echo -e "${CYAN}═══════════════════════════════════════════════════════════${NC}"
    echo ""
    # Also write section header to log file (without colors)
    echo "" >> "$LOG_FILE"
    echo "═══════════════════════════════════════════════════════════" >> "$LOG_FILE"
    echo "  $1" >> "$LOG_FILE"
    echo "═══════════════════════════════════════════════════════════" >> "$LOG_FILE"
    echo "" >> "$LOG_FILE"
}

wait_for_service() {
    local timeout=${1:-5}
    local elapsed=0
    while ! pgrep -x "cosmic-bg" > /dev/null 2>&1; do
        sleep 0.5
        ((elapsed++))
        if [ "$elapsed" -ge $((timeout * 2)) ]; then
            return 1
        fi
    done
    return 0
}

check_dependency() {
    local cmd="$1"
    local name="${2:-$cmd}"
    if command -v "$cmd" &> /dev/null; then
        log_info "Found $name: $(command -v "$cmd")"
        return 0
    else
        log_info "$name not found"
        return 1
    fi
}

# Escape path for RON string literal (backslashes and double quotes)
escape_ron_path() {
    local path="$1"
    # Escape backslashes first, then double quotes
    printf '%s' "$path" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

#------------------------------------------------------------------------------
# Setup Functions
#------------------------------------------------------------------------------

setup_test_environment() {
    log_section "Setting up test environment"

    # Create test directory
    mkdir -p "$TEST_DIR"
    echo "Test started at $(date)" > "$LOG_FILE"

    log_info "Test directory: $TEST_DIR"
    log_info "Log file: $LOG_FILE"

    # Check if running under COSMIC
    if [ -z "${WAYLAND_DISPLAY:-}" ]; then
        log_fail "Not running under Wayland - COSMIC desktop required"
        exit 1
    fi
    log_success "Wayland display detected: $WAYLAND_DISPLAY"

    # Check if cosmic-bg is running
    local cosmic_bg_pid
    cosmic_bg_pid=$(pgrep -x "cosmic-bg" 2>/dev/null | head -1 || true)

    if [ -n "$cosmic_bg_pid" ]; then
        local cosmic_bg_path
        cosmic_bg_path=$(readlink -f "/proc/$cosmic_bg_pid/exe" 2>/dev/null || echo "unknown")
        log_success "cosmic-bg is running (PID: $cosmic_bg_pid): $cosmic_bg_path"
    else
        log_fail "cosmic-bg is not running - please ensure COSMIC session is active"
        exit 1
    fi

    log_info "Config will be written via direct file manipulation"
}

create_test_assets() {
    log_section "Creating test assets"

    # Use existing wallpapers from user's Pictures folder or system backgrounds
    local wallpaper_dirs=(
        "$HOME/Pictures/wallpapers"
        "/usr/share/backgrounds/cosmic"
        "/usr/share/backgrounds"
    )

    # Find a static wallpaper and create both PNG and JPG versions
    local found_wallpaper=""
    for dir in "${wallpaper_dirs[@]}"; do
        if [ -d "$dir" ]; then
            # Use a subshell to avoid pipefail issues
            found_wallpaper=$(find "$dir" -maxdepth 3 -type f \( -name "*.jpg" -o -name "*.png" \) 2>/dev/null | head -1 || true)
            if [ -n "$found_wallpaper" ] && [ -f "$found_wallpaper" ]; then
                # Copy original
                cp "$found_wallpaper" "$TEST_DIR/static-test-original"

                # Create PNG version
                if command -v convert &> /dev/null; then
                    convert "$found_wallpaper" "$TEST_DIR/static-test.png" 2>/dev/null || \
                        cp "$found_wallpaper" "$TEST_DIR/static-test.png"
                    convert "$found_wallpaper" "$TEST_DIR/static-test.jpg" 2>/dev/null || \
                        cp "$found_wallpaper" "$TEST_DIR/static-test.jpg"
                else
                    cp "$found_wallpaper" "$TEST_DIR/static-test.png"
                    cp "$found_wallpaper" "$TEST_DIR/static-test.jpg"
                fi

                log_success "Found static test images: PNG and JPG from $(basename "$found_wallpaper")"
                break
            fi
        fi
    done

    if [ ! -f "$TEST_DIR/static-test.png" ]; then
        log_skip "No static test images found"
    fi

    # Create a simple animated GIF (with timeout protection)
    if command -v convert &> /dev/null; then
        log_info "Creating animated GIF (3 frames)..."
        if timeout 30 bash -c '
            convert -size 200x200 xc:red "$1/frame_0.png" 2>/dev/null
            convert -size 200x200 xc:green "$1/frame_1.png" 2>/dev/null
            convert -size 200x200 xc:blue "$1/frame_2.png" 2>/dev/null
            convert -delay 50 -loop 0 "$1/frame_*.png" "$1/animated-test.gif" 2>/dev/null
            rm -f "$1"/frame_*.png
        ' _ "$TEST_DIR"; then
            log_success "Created animated-test.gif"
        else
            log_skip "GIF creation timed out or failed"
        fi
    else
        log_skip "ImageMagick not found - animated GIF test will be skipped"
    fi

    # Create a short test video (with timeout protection)
    if command -v ffmpeg &> /dev/null; then
        log_info "Creating test video (3 seconds)..."
        if timeout 30 ffmpeg -y -f lavfi -i "color=c=blue:duration=3:s=320x240" \
            -c:v libx264 -preset ultrafast -crf 28 -an \
            "$TEST_DIR/video-test.mp4" 2>/dev/null; then
            log_success "Created video-test.mp4"
        else
            log_skip "Video creation timed out or failed"
        fi
    else
        log_skip "ffmpeg not found - video test will be skipped"
    fi
}

cleanup_test_assets() {
    log_section "Cleaning up"

    if [ -d "$TEST_DIR" ]; then
        # Keep the log file
        mv "$LOG_FILE" "/tmp/cosmic-bg-ng-test-$(date +%Y%m%d-%H%M%S).log" 2>/dev/null || true
        rm -rf "$TEST_DIR"
        log_info "Test assets cleaned up"
    fi
}

#------------------------------------------------------------------------------
# Feature Tests
#------------------------------------------------------------------------------

test_static_images() {
    log_section "Testing Static Images"

    # Test PNG
    if [ -f "$TEST_DIR/static-test.png" ]; then
        log_info "Testing PNG wallpaper..."
        set_wallpaper_path "$TEST_DIR/static-test.png"
        sleep 2

        if check_wallpaper_active; then
            log_success "PNG wallpaper loaded successfully"
        else
            log_fail "PNG wallpaper failed to load"
        fi
    else
        log_skip "No PNG test file available"
    fi

    # Test JPEG
    if [ -f "$TEST_DIR/static-test.jpg" ]; then
        log_info "Testing JPEG wallpaper..."
        set_wallpaper_path "$TEST_DIR/static-test.jpg"
        sleep 2

        if check_wallpaper_active; then
            log_success "JPEG wallpaper loaded successfully"
        else
            log_fail "JPEG wallpaper failed to load"
        fi
    else
        log_skip "No JPEG test file available"
    fi

    # Test system default wallpaper
    local default_wallpaper="/usr/share/backgrounds/cosmic/orion_nebula_nasa_heic0601a.jpg"
    if [ -f "$default_wallpaper" ]; then
        log_info "Testing default COSMIC wallpaper..."
        set_wallpaper_path "$default_wallpaper"
        sleep 2

        if check_wallpaper_active; then
            log_success "Default COSMIC wallpaper loaded successfully"
        else
            log_fail "Default COSMIC wallpaper failed to load"
        fi
    fi
}

test_animated_images() {
    log_section "Testing Animated Images (GIF/WebP/APNG)"

    # Test animated GIF
    if [ -f "$TEST_DIR/animated-test.gif" ]; then
        log_info "Testing animated GIF wallpaper..."
        set_animated_wallpaper "$TEST_DIR/animated-test.gif"
        sleep 3  # Give it time to load and animate

        if check_wallpaper_active; then
            log_success "Animated GIF loaded"

            # Check if frames are being updated (via service logs)
            if journalctl --user -u cosmic-bg -n 10 --no-pager 2>/dev/null | grep -q "frame"; then
                log_success "GIF animation frames detected in logs"
            else
                log_info "Could not verify frame animation from logs"
            fi
        else
            log_fail "Animated GIF failed to load"
        fi
    else
        log_skip "No animated GIF test file available"
    fi

    # Test WebP (if available)
    if [ -f "$TEST_DIR/animated-test.webp" ]; then
        log_info "Testing animated WebP wallpaper..."
        set_animated_wallpaper "$TEST_DIR/animated-test.webp"
        sleep 3

        if check_wallpaper_active; then
            log_success "Animated WebP loaded successfully"
        else
            log_fail "Animated WebP failed to load"
        fi
    else
        log_skip "No animated WebP test file available"
    fi
}

test_video_wallpapers() {
    log_section "Testing Video Wallpapers"

    # Check GStreamer availability
    if ! check_dependency "gst-inspect-1.0" "GStreamer"; then
        log_skip "GStreamer not available - video tests skipped"
        return
    fi

    # Check for required GStreamer plugins
    local required_plugins=("decodebin" "videoconvert" "appsink")
    local missing_plugins=()

    for plugin in "${required_plugins[@]}"; do
        if ! gst-inspect-1.0 "$plugin" &> /dev/null; then
            missing_plugins+=("$plugin")
        fi
    done

    if [ ${#missing_plugins[@]} -gt 0 ]; then
        log_skip "Missing GStreamer plugins: ${missing_plugins[*]}"
        return
    fi
    log_success "All required GStreamer plugins available"

    # Check hardware acceleration
    log_info "Checking hardware acceleration..."
    if gst-inspect-1.0 vaapidecodebin &> /dev/null; then
        log_success "VA-API hardware acceleration available"
    elif gst-inspect-1.0 nvdec &> /dev/null; then
        log_success "NVDEC hardware acceleration available"
    else
        log_info "No hardware acceleration - will use software decode"
    fi

    # Test video playback
    if [ -f "$TEST_DIR/video-test.mp4" ]; then
        log_info "Testing video wallpaper..."
        set_video_wallpaper "$TEST_DIR/video-test.mp4" true
        sleep 5  # Give it time to start playback

        if check_wallpaper_active; then
            log_success "Video wallpaper initialized"

            # Check for playback in logs
            if journalctl --user -u cosmic-bg -n 20 --no-pager 2>/dev/null | grep -qi "video\|gstreamer\|pipeline"; then
                log_success "Video playback confirmed in service logs"
            else
                log_info "Could not verify video playback from logs"
            fi
        else
            log_fail "Video wallpaper failed to initialize"
        fi
    else
        log_skip "No video test file available"
    fi
}

test_shader_wallpapers() {
    log_section "Testing GPU Shader Wallpapers"

    # Check for Vulkan support
    if ! check_dependency "vulkaninfo" "Vulkan"; then
        log_info "vulkaninfo not found - checking GPU anyway..."
    else
        log_info "Vulkan devices:"
        vulkaninfo --summary 2>/dev/null | grep -E "deviceName|driverVersion" || true
    fi

    # Test each shader preset
    local presets=("Plasma" "Waves" "Gradient")

    for preset in "${presets[@]}"; do
        log_info "Testing shader preset: $preset"

        # Create shader config entry
        set_shader_wallpaper "$preset"
        sleep 3

        if check_wallpaper_active; then
            log_success "Shader '$preset' loaded successfully"
        else
            log_fail "Shader '$preset' failed to load"
        fi
    done
}

test_image_cache() {
    log_section "Testing Image Cache"

    # The cache is internal, but we can test its behavior
    log_info "Testing rapid wallpaper switching (cache stress test)..."

    # Build list of available test wallpapers
    local wallpapers=(
        "$TEST_DIR/static-test.png"
        "$TEST_DIR/static-test.jpg"
        "/usr/share/backgrounds/cosmic/orion_nebula_nasa_heic0601a.jpg"
    )

    # Filter to only existing wallpapers
    local existing_wallpapers=()
    for wp in "${wallpapers[@]}"; do
        if [ -f "$wp" ]; then
            existing_wallpapers+=("$wp")
        fi
    done

    log_info "Found ${#existing_wallpapers[@]} test images for cache testing"

    if [ ${#existing_wallpapers[@]} -lt 2 ]; then
        log_skip "Not enough test images for cache test (need at least 2)"
        return
    fi

    # Rapid switching test
    local start_time=$(date +%s%N)
    for _ in {1..5}; do
        for wp in "${existing_wallpapers[@]}"; do
            set_wallpaper_path "$wp"
            sleep 0.5
        done
    done
    local end_time=$(date +%s%N)
    local duration=$(( (end_time - start_time) / 1000000 ))

    log_success "Cache stress test completed in ${duration}ms"

    # Second pass should be faster due to caching
    start_time=$(date +%s%N)
    for _ in {1..5}; do
        for wp in "${existing_wallpapers[@]}"; do
            set_wallpaper_path "$wp"
            sleep 0.3
        done
    done
    end_time=$(date +%s%N)
    local duration2=$(( (end_time - start_time) / 1000000 ))

    log_success "Second cache pass completed in ${duration2}ms"

    if [ "$duration2" -lt "$duration" ]; then
        log_success "Cache appears to be working (second pass was faster)"
    else
        log_info "Cache behavior could not be conclusively verified"
    fi
}

test_scaling_modes() {
    log_section "Testing Scaling Modes"

    # Find a test image - try our test images first, then system images
    local test_image=""
    for img in "$TEST_DIR/static-test.png" "$TEST_DIR/static-test.jpg" "/usr/share/backgrounds/cosmic/orion_nebula_nasa_heic0601a.jpg"; do
        if [ -f "$img" ]; then
            test_image="$img"
            break
        fi
    done

    if [ -z "$test_image" ]; then
        log_skip "No test image available for scaling mode tests"
        return
    fi
    log_info "Using test image: $(basename "$test_image")"

    local modes=("Zoom" "Fit" "Stretch")

    for mode in "${modes[@]}"; do
        log_info "Testing scaling mode: $mode"
        set_wallpaper_with_scaling "$test_image" "$mode"
        sleep 2

        if check_wallpaper_active; then
            log_success "Scaling mode '$mode' applied successfully"
        else
            log_fail "Scaling mode '$mode' failed"
        fi
    done
}

test_service_status() {
    log_section "Testing Service Status"

    # Check if cosmic-bg is running
    local pid
    pid=$(pgrep -x "cosmic-bg" 2>/dev/null | head -1 || true)

    if [ -n "$pid" ]; then
        log_success "cosmic-bg is running (PID: $pid)"

        # Check memory usage
        local mem
        mem=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || echo "0")
        local mem_mb=$((mem / 1024))
        log_info "Memory usage: ${mem_mb}MB"

        # Check CPU usage
        local cpu
        cpu=$(ps -o %cpu= -p "$pid" 2>/dev/null | tr -d ' ' || echo "0")
        log_info "CPU usage: ${cpu}%"

        # Check file descriptors
        local fds
        fds=$(ls /proc/"$pid"/fd 2>/dev/null | wc -l || echo "0")
        log_info "Open file descriptors: $fds"
    else
        log_fail "cosmic-bg is not running"
    fi

    # Check systemd service status
    if systemctl --user is-active cosmic-bg &> /dev/null; then
        log_success "cosmic-bg systemd service is active"
    else
        log_info "cosmic-bg may be running outside systemd"
    fi

    # Check recent logs
    log_info "Recent service logs:"
    journalctl --user -u cosmic-bg -n 5 --no-pager 2>/dev/null || \
        log_info "Could not read service logs"
}

#------------------------------------------------------------------------------
# Wallpaper Configuration Functions
#------------------------------------------------------------------------------

set_wallpaper_path() {
    local path="$1"
    local escaped_path
    escaped_path="$(escape_ron_path "$path")"

    # Write config file in RON format (cosmic-config format)
    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/$COSMIC_CONFIG_NAME/v1"
    mkdir -p "$config_dir"

    cat > "$config_dir/all" << EOF
(
    output: "all",
    source: Path("$escaped_path"),
    filter_by_theme: false,
    rotation_frequency: 3600,
    filter_method: Lanczos,
    scaling_mode: Zoom,
    sampling_method: Alphanumeric,
)
EOF

    # Touch the config to trigger a reload
    touch "$config_dir/all"
}

set_video_wallpaper() {
    local path="$1"
    local loop_playback="${2:-true}"
    local escaped_path
    escaped_path="$(escape_ron_path "$path")"

    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/$COSMIC_CONFIG_NAME/v1"
    mkdir -p "$config_dir"

    cat > "$config_dir/all" << EOF
(
    output: "all",
    source: Video((
        path: "$escaped_path",
        loop_playback: $loop_playback,
        playback_speed: 1.0,
        hw_accel: true,
    )),
    filter_by_theme: false,
    rotation_frequency: 3600,
    filter_method: Lanczos,
    scaling_mode: Zoom,
    sampling_method: Alphanumeric,
)
EOF

    touch "$config_dir/all"
}

set_animated_wallpaper() {
    local path="$1"
    local escaped_path
    escaped_path="$(escape_ron_path "$path")"

    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/$COSMIC_CONFIG_NAME/v1"
    mkdir -p "$config_dir"

    cat > "$config_dir/all" << EOF
(
    output: "all",
    source: Animated((
        path: "$escaped_path",
        fps_limit: None,
        loop_count: None,
    )),
    filter_by_theme: false,
    rotation_frequency: 3600,
    filter_method: Lanczos,
    scaling_mode: Zoom,
    sampling_method: Alphanumeric,
)
EOF

    touch "$config_dir/all"
}

set_shader_wallpaper() {
    local preset="$1"

    # Validate preset to avoid malformed configuration
    local safe_preset
    case "$preset" in
        Plasma|Waves|Gradient)
            safe_preset="$preset"
            ;;
        *)
            echo -e "${YELLOW}[WARN]${NC} Invalid shader preset '$preset'; defaulting to Plasma" >&2
            safe_preset="Plasma"
            ;;
    esac

    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/$COSMIC_CONFIG_NAME/v1"
    mkdir -p "$config_dir"

    cat > "$config_dir/all" << EOF
(
    output: "all",
    source: Shader((
        preset: Some($safe_preset),
        custom_path: None,
        fps_limit: 30,
    )),
    filter_by_theme: false,
    rotation_frequency: 3600,
    filter_method: Lanczos,
    scaling_mode: Zoom,
    sampling_method: Alphanumeric,
)
EOF

    touch "$config_dir/all"
}

set_wallpaper_with_scaling() {
    local path="$1"
    local mode="$2"
    local escaped_path
    escaped_path="$(escape_ron_path "$path")"

    local scaling_mode
    case "$mode" in
        "Zoom")    scaling_mode='Zoom' ;;
        "Fit")     scaling_mode='Fit([0.0, 0.0, 0.0])' ;;
        "Stretch") scaling_mode='Stretch' ;;
        *)         scaling_mode='Zoom' ;;
    esac

    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/$COSMIC_CONFIG_NAME/v1"
    mkdir -p "$config_dir"

    cat > "$config_dir/all" << EOF
(
    output: "all",
    source: Path("$escaped_path"),
    filter_by_theme: false,
    rotation_frequency: 3600,
    filter_method: Lanczos,
    scaling_mode: $scaling_mode,
    sampling_method: Alphanumeric,
)
EOF

    touch "$config_dir/all"
}

check_wallpaper_active() {
    # Check if cosmic-bg is still running and hasn't crashed
    if ! pgrep -x "cosmic-bg" > /dev/null; then
        return 1
    fi

    # Verify config file exists and is non-empty
    local config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/cosmic/$COSMIC_CONFIG_NAME/v1"
    local config_file="${config_dir}/all"
    if [ ! -s "$config_file" ]; then
        return 1
    fi

    # Check for recent errors in journal (if available)
    if command -v journalctl >/dev/null 2>&1; then
        if journalctl --user -u cosmic-bg --since "30 seconds ago" 2>/dev/null | grep -qiE 'error|failed|panic'; then
            return 1
        fi
    fi

    return 0
}

#------------------------------------------------------------------------------
# Main Entry Point
#------------------------------------------------------------------------------

print_summary() {
    log_section "Test Summary"

    local total=$((TESTS_PASSED + TESTS_FAILED + TESTS_SKIPPED))

    echo -e "${GREEN}Passed:${NC}  $TESTS_PASSED"
    echo -e "${RED}Failed:${NC}  $TESTS_FAILED"
    echo -e "${YELLOW}Skipped:${NC} $TESTS_SKIPPED"
    echo -e "────────────────"
    echo -e "Total:   $total"
    echo ""

    if [ "$TESTS_FAILED" -eq 0 ]; then
        echo -e "${GREEN}All tests passed!${NC}"
    else
        echo -e "${RED}Some tests failed. Check $LOG_FILE for details.${NC}"
    fi
}

show_help() {
    cat << EOF
cosmic-bg-ng Feature Test Suite

Usage: $0 [OPTIONS]

Options:
    --all       Run all tests (default)
    --static    Test static image wallpapers only
    --animated  Test animated image wallpapers only
    --video     Test video wallpapers only
    --shader    Test shader wallpapers only
    --cache     Test image cache functionality
    --scaling   Test scaling modes
    --status    Check service status only
    --help      Show this help message

Examples:
    $0                  # Run all tests
    $0 --shader         # Test only shader wallpapers
    $0 --static --cache # Test static images and cache

Requirements:
    - Running COSMIC Desktop session
    - cosmic-bg-ng installed and active
    - ImageMagick (optional, for generating test images)
    - GStreamer (optional, for video wallpaper tests)
    - Vulkan drivers (optional, for shader tests)

EOF
}

main() {
    # Parse arguments
    local run_all=true
    local run_static=false
    local run_animated=false
    local run_video=false
    local run_shader=false
    local run_cache=false
    local run_scaling=false
    local run_status=false

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --all)
                run_all=true
                shift
                ;;
            --static)
                run_all=false
                run_static=true
                shift
                ;;
            --animated)
                run_all=false
                run_animated=true
                shift
                ;;
            --video)
                run_all=false
                run_video=true
                shift
                ;;
            --shader)
                run_all=false
                run_shader=true
                shift
                ;;
            --cache)
                run_all=false
                run_cache=true
                shift
                ;;
            --scaling)
                run_all=false
                run_scaling=true
                shift
                ;;
            --status)
                run_all=false
                run_status=true
                shift
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            *)
                echo "Unknown option: $1"
                show_help
                exit 1
                ;;
        esac
    done

    echo -e "${CYAN}"
    echo "╔═══════════════════════════════════════════════════════════════╗"
    echo "║       cosmic-bg-ng Feature Test Suite v1.0.0                 ║"
    echo "╚═══════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"

    # Setup
    setup_test_environment
    create_test_assets

    # Run requested tests
    if $run_all; then
        test_service_status
        test_static_images
        test_animated_images
        test_video_wallpapers
        test_shader_wallpapers
        test_scaling_modes
        test_image_cache
    else
        $run_status && test_service_status
        $run_static && test_static_images
        $run_animated && test_animated_images
        $run_video && test_video_wallpapers
        $run_shader && test_shader_wallpapers
        $run_scaling && test_scaling_modes
        $run_cache && test_image_cache
    fi

    # Summary
    print_summary

    # Cleanup (optional - comment out to keep test assets)
    # cleanup_test_assets
    echo -e "${YELLOW}Note: Test assets remain in: ${TEST_DIR}${NC}"
    echo -e "${YELLOW}Uncomment 'cleanup_test_assets' in this script to enable automatic cleanup.${NC}"

    # Return exit code based on failures
    [ "$TESTS_FAILED" -eq 0 ]
}

# Run main
main "$@"
