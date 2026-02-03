// SPDX-License-Identifier: MPL-2.0

//! cosmic-bg-settings - GUI application for managing COSMIC desktop wallpapers
//!
//! This application provides a graphical interface for configuring wallpapers,
//! including static images, videos, animated images, and GPU shaders.

mod app;
mod config;
mod i18n;
mod message;
mod pages;
mod widgets;

use app::App;

fn main() -> cosmic::iced::Result {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Initialize localization
    i18n::init();

    // Run the application
    cosmic::app::run::<App>(cosmic::app::Settings::default(), ())
}
