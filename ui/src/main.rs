mod activity;
mod app;
mod content;
mod icon;
mod init_workflows;
mod shortcut;
mod sidebar;
mod status_panel;
mod theme;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> iced::Result {
    env_logger::init();

    let has_wayland_socket = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    if !has_wayland_socket {
        eprintln!("Updater requires a native Wayland session.");
        std::process::exit(1);
    }

    iced::application(app::App::new, app::App::update, app::App::view)
        .font(theme::GEIST_REGULAR_BYTES)
        .font(theme::GEIST_SEMIBOLD_BYTES)
        .font(theme::GEIST_MONO_REGULAR_BYTES)
        .default_font(theme::FONT_REGULAR)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .window(iced::window::Settings {
            size: iced::Size::new(1200.0, 800.0),
            min_size: Some(iced::Size::new(640.0, 520.0)),
            exit_on_close_request: false,
            ..Default::default()
        })
        .run()
}
