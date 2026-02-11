use iced::Theme;

use crate::app::App;

mod app;
mod content;
mod icon;
mod sidebar;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn theme(_: &App) -> Theme {
    Theme::CatppuccinLatte
}

fn main() -> iced::Result {
    env_logger::init();

    iced::application(app::App::new, app::App::update, app::App::view)
        .theme(theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1200.0, 800.0),
            min_size: Some(iced::Size::new(900.0, 600.0)),
            ..Default::default()
        })
        .run()
}
