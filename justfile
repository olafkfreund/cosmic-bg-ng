name := 'cosmic-bg'
export APPID := 'com.system76.CosmicBackground'

# Use mold linker if clang and mold exists.
clang-path := `which clang || true`
mold-path := `which mold || true`

linker-arg := if clang-path != '' {
    if mold-path != '' {
        '-C linker=' + clang-path + ' -C link-arg=--ld-path=' + mold-path + ' '
    } else {
        ''
    }
} else {
    ''
}

export RUSTFLAGS := linker-arg + env_var_or_default('RUSTFLAGS', '')

rootdir := ''
prefix := '/usr'


base-dir := absolute_path(clean(rootdir / prefix))

export INSTALL_DIR := base-dir / 'share'

cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-src := cargo-target-dir / 'release' / name
bin-dst := base-dir / 'bin' / name
ctl-src := cargo-target-dir / 'release' / 'cosmic-bg-ctl'
ctl-dst := base-dir / 'bin' / 'cosmic-bg-ctl'
settings-src := cargo-target-dir / 'release' / 'cosmic-bg-settings'
settings-dst := base-dir / 'bin' / 'cosmic-bg-settings'

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# `cargo clean` and removes vendored dependencies
clean-dist: clean
    rm -rf .cargo vendor vendor.tar

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Compiles release profile with vendored dependencies
build-vendored *args: vendor-extract (build-release '--frozen --offline' args)

# Build only the CLI tool
build-ctl *args:
    cargo build --release --bin cosmic-bg-ctl {{args}}

# Build only the settings GUI
build-settings *args:
    cargo build --release -p cosmic-bg-settings {{args}}

# Build all tools (service, CLI, and GUI)
build-all *args: build-release build-settings

# Runs a clippy check
check *args:
    cargo clippy --all-features {{args}} -- -W clippy::pedantic

# Runs a clippy check with JSON message format
check-json: (check '--message-format=json')

# Check all workspace members
check-all *args:
    cargo clippy --workspace --all-features {{args}} -- -W clippy::pedantic

# Run with debug logs
run *args:
    env RUST_LOG=debug RUST_BACKTRACE=1 cargo run --release {{args}}

# Run the CLI tool
run-ctl *args:
    env RUST_LOG=debug cargo run --release --bin cosmic-bg-ctl -- {{args}}

# Generate shell completions for cosmic-bg-ctl
completions:
    cargo run --release --bin cosmic-bg-ctl -- completions bash > cosmic-bg-ctl.bash
    cargo run --release --bin cosmic-bg-ctl -- completions zsh > _cosmic-bg-ctl
    cargo run --release --bin cosmic-bg-ctl -- completions fish > cosmic-bg-ctl.fish
    @echo "Generated completions: cosmic-bg-ctl.bash, _cosmic-bg-ctl, cosmic-bg-ctl.fish"

# Run the settings GUI
run-settings *args:
    env RUST_LOG=debug cargo run --release -p cosmic-bg-settings {{args}}

# Installs files
install:
    install -Dm0755 {{bin-src}} {{bin-dst}}
    @just data/install
    @just data/icons/install

# Install the CLI tool
install-ctl:
    install -Dm0755 {{ctl-src}} {{ctl-dst}}

# Install the settings GUI
install-settings:
    install -Dm0755 {{settings-src}} {{settings-dst}}
    @just data/install-settings

# Install all binaries
install-all: install install-ctl install-settings

# Uninstalls installed files
uninstall:
    rm {{bin-dst}}
    @just data/uninstall
    @just data/icons/uninstall

# Uninstall the CLI tool
uninstall-ctl:
    rm {{ctl-dst}}

# Uninstall the settings GUI
uninstall-settings:
    rm {{settings-dst}}
    @just data/uninstall-settings

# Uninstall all binaries
uninstall-all: uninstall uninstall-ctl uninstall-settings

# Vendor dependencies locally
vendor:
    mkdir -p .cargo
    cargo vendor --sync Cargo.toml --sync config/Cargo.toml --sync cosmic-bg-settings/Cargo.toml | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    tar pcf vendor.tar vendor
    rm -rf vendor

# Extracts vendored dependencies
vendor-extract:
    #!/usr/bin/env sh
    rm -rf vendor
    tar pxf vendor.tar
