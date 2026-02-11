use std::sync::LazyLock;

use iced::advanced::svg;

pub static SAVE_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/save.svg").to_vec())
});

pub static ADD_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/add.svg").to_vec())
});

pub static REFRESH_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/refresh.svg").to_vec())
});

pub static FIND_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/find.svg").to_vec())
});

pub static UPDATE_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/update.svg").to_vec())
});

pub static INSTALLED_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/installed.svg").to_vec())
});

pub static SETTINGS_ICON: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(include_bytes!("../../assets/icons/settings.svg").to_vec())
});
