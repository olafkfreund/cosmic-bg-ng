{
  description = "Next-generation background service for the COSMIC desktop environment with animated, video, and shader wallpaper support";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nix-filter.url = "github:numtide/nix-filter";
    crane.url = "github:ipetkov/crane";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, nix-filter, crane, fenix }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
    in
    flake-utils.lib.eachSystem supportedSystems (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        # Use latest Rust toolchain for 2024 edition support
        craneLib = (crane.mkLib pkgs).overrideToolchain fenix.packages.${system}.latest.toolchain;

        pkgDef = {
          pname = "cosmic-bg-ng";
          version = "1.0.0";

          src = nix-filter.lib.filter {
            root = ./.;
            exclude = [
              ./.gitignore
              ./flake.nix
              ./flake.lock
              ./LICENSE.md
              ./debian
              ./nix
              ./docs
            ];
          };

          nativeBuildInputs = with pkgs; [
            just
            pkg-config
            autoPatchelfHook
          ];

          buildInputs = with pkgs; [
            wayland
            libxkbcommon
            desktop-file-utils
            stdenv.cc.cc.lib
            # GStreamer for video wallpaper support
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
            gst_all_1.gst-plugins-bad
            gst_all_1.gst-plugins-ugly
            gst_all_1.gst-libav
            # Hardware acceleration support
            libva
            # wgpu/vulkan for shader support
            vulkan-loader
            vulkan-headers
          ];

          runtimeDependencies = with pkgs; [
            wayland
            vulkan-loader
            gst_all_1.gstreamer
            gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly pkgDef;

        cosmic-bg-ng = craneLib.buildPackage (pkgDef // {
          inherit cargoArtifacts;
          # Skip tests in sandbox - GStreamer/Wayland not available
          doCheck = false;
        });
      in {
        checks = {
          inherit cosmic-bg-ng;
        };

        packages = {
          default = cosmic-bg-ng.overrideAttrs (oldAttrs: {
            buildPhase = ''
              just prefix=$out build-release
            '';
            installPhase = ''
              just prefix=$out install
            '';
          });
          cosmic-bg-ng = self.packages.${system}.default;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
        };

        devShells.default = pkgs.mkShell rec {
          inputsFrom = builtins.attrValues self.checks.${system};

          nativeBuildInputs = with pkgs; [
            just
            pkg-config
            rust-analyzer
            clippy
            rustfmt
          ];

          LD_LIBRARY_PATH = pkgs.lib.strings.makeLibraryPath (
            builtins.concatMap (d: d.runtimeDependencies or []) inputsFrom
            ++ (with pkgs; [
              wayland
              vulkan-loader
              gst_all_1.gstreamer
            ])
          );

          # GStreamer plugin path for development
          GST_PLUGIN_SYSTEM_PATH_1_0 = pkgs.lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" (with pkgs.gst_all_1; [
            gstreamer
            gst-plugins-base
            gst-plugins-good
            gst-plugins-bad
            gst-plugins-ugly
            gst-libav
          ]);
        };
      }) // {
      # NixOS modules for system integration
      nixosModules = {
        default = import ./nix/module.nix;
        cosmic-bg-ng = import ./nix/module.nix;
      };

      # Overlays for package substitution
      overlays = {
        default = final: prev: {
          cosmic-bg = self.packages.${prev.system}.default;
          cosmic-bg-ng = self.packages.${prev.system}.default;
        };
      };
    };

  nixConfig = {
    # Cache for the Rust toolchain in fenix
    extra-substituters = [ "https://nix-community.cachix.org" ];
    extra-trusted-public-keys = [ "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs=" ];
  };
}
