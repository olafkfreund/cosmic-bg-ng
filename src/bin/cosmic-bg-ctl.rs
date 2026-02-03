// SPDX-License-Identifier: MPL-2.0

//! cosmic-bg-ctl - CLI tool for managing cosmic-bg wallpapers
//!
//! This tool allows setting wallpapers from the command line, including
//! static images, videos, animated images, and GPU shaders.

use std::io;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use cosmic_bg_config::{
    AnimatedConfig, Color, Context, Entry, Gradient, ScalingMode, ShaderConfig, ShaderPreset,
    Source, VideoConfig,
};

/// CLI tool for managing cosmic-bg wallpapers
#[derive(Parser)]
#[command(name = "cosmic-bg-ctl")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Set a static image wallpaper
    Set {
        /// Path to image file or directory
        path: PathBuf,
        /// Target output (e.g., DP-1, HDMI-A-1). Defaults to "all"
        #[arg(short, long)]
        output: Option<String>,
        /// Scaling mode: zoom, fit, stretch
        #[arg(short, long, default_value = "zoom")]
        scaling: String,
        /// Rotation frequency in seconds (for directories)
        #[arg(short, long)]
        rotation: Option<u64>,
    },

    /// Set a video wallpaper
    Video {
        /// Path to video file
        path: PathBuf,
        /// Target output (e.g., DP-1, HDMI-A-1). Defaults to "all"
        #[arg(short, long)]
        output: Option<String>,
        /// Loop playback (default: true)
        #[arg(long, default_value = "true")]
        r#loop: bool,
        /// Playback speed multiplier (default: 1.0)
        #[arg(long)]
        speed: Option<f64>,
        /// Disable hardware acceleration
        #[arg(long)]
        no_hw_accel: bool,
    },

    /// Set an animated image wallpaper (GIF, WebP, APNG)
    Animated {
        /// Path to animated image file
        path: PathBuf,
        /// Target output (e.g., DP-1, HDMI-A-1). Defaults to "all"
        #[arg(short, long)]
        output: Option<String>,
        /// FPS limit (default: use source FPS)
        #[arg(long)]
        fps: Option<u32>,
        /// Loop count (default: infinite)
        #[arg(long)]
        loops: Option<u32>,
    },

    /// Set a GPU shader wallpaper
    Shader {
        /// Shader preset name (Plasma, Waves, Gradient) or path to custom .wgsl file
        preset_or_path: String,
        /// Target output (e.g., DP-1, HDMI-A-1). Defaults to "all"
        #[arg(short, long)]
        output: Option<String>,
        /// Target FPS (default: 30)
        #[arg(long, default_value = "30")]
        fps: u32,
    },

    /// Set a solid color or gradient wallpaper
    Color {
        /// Color in hex format (e.g., #ff0000) or "gradient"
        color: String,
        /// Additional colors for gradient (space-separated hex values)
        #[arg(long)]
        gradient_colors: Option<Vec<String>>,
        /// Gradient radius (0.0-1.0, default: 0.5)
        #[arg(long, default_value = "0.5")]
        radius: f32,
        /// Target output (e.g., DP-1, HDMI-A-1). Defaults to "all"
        #[arg(short, long)]
        output: Option<String>,
    },

    /// Query current wallpaper configuration
    Query {
        /// Target output (e.g., DP-1). If not specified, shows all
        #[arg(short, long)]
        output: Option<String>,
    },

    /// List available display outputs
    Outputs,

    /// Backup current configuration
    Backup {
        /// Output file path
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Restore configuration from backup
    Restore {
        /// Input file path
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Generate shell completions
    Completions {
        /// Shell type: bash, zsh, fish
        shell: String,
    },
}

fn main() {
    // Initialize tracing with simple format
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let context = cosmic_bg_config::context()?;

    match cli.command {
        Commands::Set {
            path,
            output,
            scaling,
            rotation,
        } => cmd_set(&context, path, output, &scaling, rotation),
        Commands::Video {
            path,
            output,
            r#loop,
            speed,
            no_hw_accel,
        } => cmd_video(&context, path, output, r#loop, speed, no_hw_accel),
        Commands::Animated {
            path,
            output,
            fps,
            loops,
        } => cmd_animated(&context, path, output, fps, loops),
        Commands::Shader {
            preset_or_path,
            output,
            fps,
        } => cmd_shader(&context, preset_or_path, output, fps),
        Commands::Color {
            color,
            gradient_colors,
            radius,
            output,
        } => cmd_color(&context, color, gradient_colors, radius, output),
        Commands::Query { output } => cmd_query(&context, output),
        Commands::Outputs => cmd_outputs(&context),
        Commands::Backup { file } => cmd_backup(&context, file),
        Commands::Restore { file } => cmd_restore(&context, file),
        Commands::Completions { shell } => cmd_completions(&shell),
    }
}

fn parse_scaling_mode(scaling: &str) -> Result<ScalingMode, Box<dyn std::error::Error>> {
    match scaling.to_lowercase().as_str() {
        "zoom" => Ok(ScalingMode::Zoom),
        "stretch" => Ok(ScalingMode::Stretch),
        "fit" => Ok(ScalingMode::Fit([0.0, 0.0, 0.0])), // Black background
        _ => Err(format!("Unknown scaling mode: {scaling}. Use: zoom, fit, stretch").into()),
    }
}

fn parse_hex_color(hex: &str) -> Result<[f32; 3], Box<dyn std::error::Error>> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return Err("Invalid hex color format. Use: #rrggbb".into());
    }

    let r = u8::from_str_radix(&hex[0..2], 16)? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16)? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16)? as f32 / 255.0;

    Ok([r, g, b])
}

fn cmd_set(
    context: &Context,
    path: PathBuf,
    output: Option<String>,
    scaling: &str,
    rotation: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.canonicalize().map_err(|e| format!("Invalid path: {e}"))?;

    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()).into());
    }

    let output_name = output.unwrap_or_else(|| "all".to_string());
    let scaling_mode = parse_scaling_mode(scaling)?;

    let mut entry = Entry::new(output_name.clone(), Source::Path(path.clone()));
    entry.scaling_mode = scaling_mode;
    if let Some(freq) = rotation {
        entry.rotation_frequency = freq;
    }

    let mut config = cosmic_bg_config::Config::load(context)?;
    config.set_entry(context, entry)?;

    println!("Set wallpaper for '{output_name}': {}", path.display());
    Ok(())
}

fn cmd_video(
    context: &Context,
    path: PathBuf,
    output: Option<String>,
    loop_playback: bool,
    speed: Option<f64>,
    no_hw_accel: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.canonicalize().map_err(|e| format!("Invalid path: {e}"))?;

    if !path.is_file() {
        return Err(format!("Video file does not exist: {}", path.display()).into());
    }

    let output_name = output.unwrap_or_else(|| "all".to_string());

    let video_config = VideoConfig {
        path: path.clone(),
        loop_playback,
        playback_speed: speed.unwrap_or(1.0),
        hw_accel: !no_hw_accel,
    };

    let entry = Entry::new(output_name.clone(), Source::Video(video_config));

    let mut config = cosmic_bg_config::Config::load(context)?;
    config.set_entry(context, entry)?;

    println!("Set video wallpaper for '{output_name}': {}", path.display());
    if !loop_playback {
        println!("  Loop: disabled");
    }
    if let Some(s) = speed {
        println!("  Speed: {s}x");
    }
    if no_hw_accel {
        println!("  Hardware acceleration: disabled");
    }
    Ok(())
}

fn cmd_animated(
    context: &Context,
    path: PathBuf,
    output: Option<String>,
    fps: Option<u32>,
    loops: Option<u32>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.canonicalize().map_err(|e| format!("Invalid path: {e}"))?;

    if !path.is_file() {
        return Err(format!("Animated image file does not exist: {}", path.display()).into());
    }

    let output_name = output.unwrap_or_else(|| "all".to_string());

    let animated_config = AnimatedConfig {
        path: path.clone(),
        fps_limit: fps,
        loop_count: loops,
    };

    let entry = Entry::new(output_name.clone(), Source::Animated(animated_config));

    let mut config = cosmic_bg_config::Config::load(context)?;
    config.set_entry(context, entry)?;

    println!(
        "Set animated wallpaper for '{output_name}': {}",
        path.display()
    );
    if let Some(f) = fps {
        println!("  FPS limit: {f}");
    }
    if let Some(l) = loops {
        println!("  Loop count: {l}");
    }
    Ok(())
}

fn cmd_shader(
    context: &Context,
    preset_or_path: String,
    output: Option<String>,
    fps: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_name = output.unwrap_or_else(|| "all".to_string());

    // Check if it's a preset or a file path
    let shader_config = match preset_or_path.to_lowercase().as_str() {
        "plasma" => ShaderConfig {
            preset: Some(ShaderPreset::Plasma),
            custom_path: None,
            fps_limit: fps,
        },
        "waves" => ShaderConfig {
            preset: Some(ShaderPreset::Waves),
            custom_path: None,
            fps_limit: fps,
        },
        "gradient" => ShaderConfig {
            preset: Some(ShaderPreset::Gradient),
            custom_path: None,
            fps_limit: fps,
        },
        _ => {
            // Assume it's a file path
            let path = PathBuf::from(&preset_or_path);
            let path = path.canonicalize().map_err(|e| format!("Invalid shader path: {e}"))?;

            if !path.is_file() {
                return Err(format!("Shader file does not exist: {}", path.display()).into());
            }

            ShaderConfig {
                preset: None,
                custom_path: Some(path),
                fps_limit: fps,
            }
        }
    };

    let entry = Entry::new(output_name.clone(), Source::Shader(shader_config.clone()));

    let mut config = cosmic_bg_config::Config::load(context)?;
    config.set_entry(context, entry)?;

    if let Some(preset) = &shader_config.preset {
        println!("Set shader wallpaper for '{output_name}': {preset:?}");
    } else if let Some(path) = &shader_config.custom_path {
        println!(
            "Set custom shader wallpaper for '{output_name}': {}",
            path.display()
        );
    }
    println!("  FPS limit: {fps}");
    Ok(())
}

fn cmd_color(
    context: &Context,
    color: String,
    gradient_colors: Option<Vec<String>>,
    radius: f32,
    output: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_name = output.unwrap_or_else(|| "all".to_string());

    let source = if let Some(colors) = gradient_colors {
        // Gradient mode
        let mut all_colors = vec![parse_hex_color(&color)?];
        for c in &colors {
            all_colors.push(parse_hex_color(c)?);
        }

        let gradient = Gradient {
            colors: all_colors.into(),
            radius,
        };

        Source::Color(Color::Gradient(gradient))
    } else {
        // Single color
        Source::Color(Color::Single(parse_hex_color(&color)?))
    };

    let entry = Entry::new(output_name.clone(), source.clone());

    let mut config = cosmic_bg_config::Config::load(context)?;
    config.set_entry(context, entry)?;

    match source {
        Source::Color(Color::Single(rgb)) => {
            println!(
                "Set solid color wallpaper for '{output_name}': #{:02x}{:02x}{:02x}",
                (rgb[0] * 255.0) as u8,
                (rgb[1] * 255.0) as u8,
                (rgb[2] * 255.0) as u8
            );
        }
        Source::Color(Color::Gradient(g)) => {
            println!("Set gradient wallpaper for '{output_name}':");
            for c in g.colors.iter() {
                println!(
                    "  - #{:02x}{:02x}{:02x}",
                    (c[0] * 255.0) as u8,
                    (c[1] * 255.0) as u8,
                    (c[2] * 255.0) as u8
                );
            }
            println!("  Radius: {radius}");
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn cmd_query(context: &Context, output: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let config = cosmic_bg_config::Config::load(context)?;

    println!("Wallpaper Configuration");
    println!("=======================");
    println!("Same on all displays: {}", config.same_on_all);
    println!();

    if let Some(output_name) = output {
        // Show specific output
        if output_name == "all" {
            print_entry(&config.default_background);
        } else if let Some(entry) = config.entry(&output_name) {
            print_entry(entry);
        } else {
            println!("No specific configuration for '{output_name}', using default:");
            print_entry(&config.default_background);
        }
    } else {
        // Show all
        println!("Default (all displays):");
        print_entry(&config.default_background);

        if !config.backgrounds.is_empty() {
            println!("\nPer-output configurations:");
            for entry in &config.backgrounds {
                println!();
                print_entry(entry);
            }
        }
    }

    Ok(())
}

fn print_entry(entry: &Entry) {
    println!("  Output: {}", entry.output);
    match &entry.source {
        Source::Path(path) => println!("  Type: Static image\n  Path: {}", path.display()),
        Source::Color(Color::Single(rgb)) => println!(
            "  Type: Solid color\n  Color: #{:02x}{:02x}{:02x}",
            (rgb[0] * 255.0) as u8,
            (rgb[1] * 255.0) as u8,
            (rgb[2] * 255.0) as u8
        ),
        Source::Color(Color::Gradient(g)) => {
            println!("  Type: Gradient");
            for c in g.colors.iter() {
                println!(
                    "    - #{:02x}{:02x}{:02x}",
                    (c[0] * 255.0) as u8,
                    (c[1] * 255.0) as u8,
                    (c[2] * 255.0) as u8
                );
            }
        }
        Source::Video(v) => {
            println!("  Type: Video\n  Path: {}", v.path.display());
            println!("  Loop: {}", v.loop_playback);
            println!("  Speed: {}x", v.playback_speed);
            println!("  HW Accel: {}", v.hw_accel);
        }
        Source::Animated(a) => {
            println!("  Type: Animated image\n  Path: {}", a.path.display());
            if let Some(fps) = a.fps_limit {
                println!("  FPS limit: {fps}");
            }
            if let Some(loops) = a.loop_count {
                println!("  Loop count: {loops}");
            }
        }
        Source::Shader(s) => {
            println!("  Type: GPU Shader");
            if let Some(preset) = &s.preset {
                println!("  Preset: {preset:?}");
            }
            if let Some(path) = &s.custom_path {
                println!("  Custom: {}", path.display());
            }
            println!("  FPS limit: {}", s.fps_limit);
        }
    }
    println!("  Scaling: {:?}", entry.scaling_mode);
    println!("  Rotation frequency: {}s", entry.rotation_frequency);
}

fn cmd_outputs(context: &Context) -> Result<(), Box<dyn std::error::Error>> {
    let config = cosmic_bg_config::Config::load(context)?;

    println!("Configured outputs:");
    println!("  all (default)");

    for output in &config.outputs {
        println!("  {output}");
    }

    println!("\nNote: To see all connected displays, run cosmic-bg with debug logging.");
    Ok(())
}

fn cmd_backup(
    context: &Context,
    file: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = cosmic_bg_config::Config::load(context)?;

    let backup_file = file.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/cosmic-bg-backup.ron")
    });

    let ron_str = ron::ser::to_string_pretty(&config.backgrounds, ron::ser::PrettyConfig::default())?;
    std::fs::write(&backup_file, ron_str)?;

    println!("Configuration backed up to: {}", backup_file.display());
    Ok(())
}

fn cmd_completions(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    let shell = match shell.to_lowercase().as_str() {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        "fish" => Shell::Fish,
        "elvish" => Shell::Elvish,
        "powershell" => Shell::PowerShell,
        _ => {
            return Err(format!(
                "Unsupported shell: {shell}. Supported: bash, zsh, fish, elvish, powershell"
            )
            .into())
        }
    };

    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "cosmic-bg-ctl", &mut io::stdout());
    Ok(())
}

fn cmd_restore(
    context: &Context,
    file: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let backup_file = file.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".config/cosmic-bg-backup.ron")
    });

    if !backup_file.exists() {
        return Err(format!("Backup file not found: {}", backup_file.display()).into());
    }

    let ron_str = std::fs::read_to_string(&backup_file)?;
    let backgrounds: Vec<Entry> = ron::from_str(&ron_str)?;

    let mut config = cosmic_bg_config::Config::load(context)?;

    for entry in backgrounds {
        config.set_entry(context, entry)?;
    }

    println!("Configuration restored from: {}", backup_file.display());
    Ok(())
}
