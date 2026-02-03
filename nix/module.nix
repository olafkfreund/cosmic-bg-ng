{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.cosmic-bg-ng;
in
{
  options.services.cosmic-bg-ng = {
    enable = mkEnableOption "COSMIC Background NG daemon as a replacement for the default COSMIC background service";

    package = mkPackageOption pkgs "cosmic-bg-ng" {
      default = [ "cosmic-bg-ng" ];
      example = literalExpression "pkgs.cosmic-bg-ng";
    };

    replaceSystemPackage = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Replace the system cosmic-bg package with cosmic-bg-ng.

        This creates an overlay that substitutes the default COSMIC background
        daemon with this enhanced version across the entire system.
      '';
    };

    settings = {
      enableVideoWallpapers = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Enable video wallpaper support (MP4, WebM).

          Requires GStreamer plugins to be available.
        '';
      };

      enableShaderWallpapers = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Enable GPU shader wallpaper support.

          Requires a GPU with Vulkan, Metal, or DX12 support.
        '';
      };

      enableAnimatedWallpapers = mkOption {
        type = types.bool;
        default = true;
        description = ''
          Enable animated image support (GIF, APNG, WebP).
        '';
      };

      maxCacheSize = mkOption {
        type = types.ints.positive;
        default = 512;
        example = 1024;
        description = ''
          Maximum image cache size in megabytes.

          Higher values reduce memory reloading when switching between
          wallpapers but consume more RAM.
        '';
      };

      maxCacheEntries = mkOption {
        type = types.ints.positive;
        default = 50;
        example = 100;
        description = ''
          Maximum number of images to keep in cache.
        '';
      };
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.settings.maxCacheSize > 0;
        message = "services.cosmic-bg-ng.settings.maxCacheSize must be positive";
      }
      {
        assertion = cfg.settings.maxCacheEntries > 0;
        message = "services.cosmic-bg-ng.settings.maxCacheEntries must be positive";
      }
    ];

    warnings = optional (!cfg.settings.enableVideoWallpapers && !cfg.settings.enableShaderWallpapers && !cfg.settings.enableAnimatedWallpapers) [
      "All advanced wallpaper features are disabled in cosmic-bg-ng. Consider enabling at least one feature type."
    ];

    # Replace the system cosmic-bg package with cosmic-bg-ng
    nixpkgs.overlays = mkIf cfg.replaceSystemPackage [
      (final: prev: {
        cosmic-bg = cfg.package;
      })
    ];

    # If not replacing, add to system packages
    environment.systemPackages = mkIf (!cfg.replaceSystemPackage) [ cfg.package ];

    # Ensure GStreamer plugins are available for video wallpapers
    environment.sessionVariables = mkIf cfg.settings.enableVideoWallpapers {
      GST_PLUGIN_SYSTEM_PATH_1_0 = lib.makeSearchPathOutput "lib" "lib/gstreamer-1.0" (with pkgs.gst_all_1; [
        gstreamer
        gst-plugins-base
        gst-plugins-good
        gst-plugins-bad
        gst-plugins-ugly
        gst-libav
      ]);
    };

    # Add hardware acceleration packages for video decoding
    hardware.opengl = mkIf cfg.settings.enableVideoWallpapers {
      enable = mkDefault true;
      extraPackages = with pkgs; [
        libva
        vaapiVdpau
        libvdpau-va-gl
      ];
    };

    meta.maintainers = with maintainers; [ ];
  };
}
