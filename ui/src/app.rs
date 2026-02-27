use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::join_all;
use iced::{Animation, Length, Subscription, Task};
use updater_core::{PackageManagerType, PackageUpdate};

use crate::{
    content::{self, Content, FindingInfo, InstalledInfo, UpdatesInfo},
    sidebar::{self, SideBar},
};

#[allow(dead_code)]
pub mod colors {
    use iced::Color;

    // Primary - Light Green
    pub const PRIMARY: Color = Color::from_rgb8(211, 249, 216); // #d3f9d8
    pub const PRIMARY_HOVER: Color = Color::from_rgb8(196, 240, 204); // #c4f0cc
    pub const PRIMARY_ACTIVE: Color = Color::from_rgb8(173, 231, 190); // #ade7be
    pub const PRIMARY_LIGHT: Color = Color::from_rgb8(228, 252, 228); // #e4fce4
    pub const PRIMARY_MUTED: Color = Color::from_rgb8(180, 220, 180); // #b4dcb4

    // Secondary - Cyan Blue
    pub const SECONDARY: Color = Color::from_rgb8(59, 201, 219); // #3bc9db
    pub const SECONDARY_HOVER: Color = Color::from_rgb8(34, 184, 207); // #22b8cf
    pub const SECONDARY_ACTIVE: Color = Color::from_rgb8(21, 170, 191); // #15aabf
    pub const SECONDARY_SOFT: Color = Color::from_rgb8(150, 219, 230); // #96dbe6

    // Surface
    pub const SURFACE: Color = Color::from_rgb8(247, 248, 250); // #f7f8fa
    pub const SURFACE_HOVER: Color = Color::from_rgb8(238, 240, 243); // #eef0f3
    pub const SURFACE_PRESSED: Color = Color::from_rgb8(222, 226, 230); // #dee2e6
    pub const SURFACE_MUTED: Color = Color::from_rgb8(245, 246, 248); // #f5f6f8
    pub const SURFACE_ALT: Color = Color::from_rgb8(250, 251, 253); // #fafbfd

    // Foreground
    pub const ON_PRIMARY: Color = Color::from_rgb8(34, 52, 40); // #223428
    pub const ON_SURFACE: Color = Color::from_rgb8(52, 58, 64); // #343a40
    pub const ON_SURFACE_IDLE: Color = Color::from_rgb8(77, 85, 92); // #4d555c
    pub const ON_SURFACE_MUTED: Color = Color::from_rgb8(130, 138, 145); // #828a91
    pub const ON_SURFACE_ALT: Color = Color::from_rgb8(95, 102, 110); // #5f666e

    // Accent
    pub const ACCENT: Color = Color::from_rgb8(173, 231, 190); // #ade7be
    pub const ACCENT_HOVER: Color = Color::from_rgb8(150, 210, 170); // #96d2aa
    pub const ACCENT_MUTED: Color = Color::from_rgb8(200, 240, 210); // #c8f0d2

    // Status
    pub const SUCCESS: Color = Color::from_rgb8(76, 175, 80); // #4caf50
    pub const WARNING: Color = Color::from_rgb8(255, 193, 7); // #ffc107
    pub const ERROR: Color = Color::from_rgb8(244, 67, 54); // #f44336

    // Helpers
    pub const DIVIDER: Color = Color::from_rgb8(220, 224, 228); // #dce0e4
    pub const DIVIDER_LIGHT: Color = Color::from_rgb8(235, 238, 242); // #ebeef2
    pub const SHADOW: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.08);
    pub const SHADOW_LIGHT: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.05);
    pub const SHADOW_HEAVY: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.2);
    pub const OVERLAY: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.3);
    pub const OVERLAY_LIGHT: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.08);
    pub const FOCUS: Color = Color::from_rgb8(173, 231, 190); // #ade7be
    pub const DISABLED: Color = Color::from_rgb8(200, 205, 210); // #c8cdd2
}

enum ProgressMode {
    Determinate(f32),
    Indeterminate,
}

#[derive(Debug, Clone)]
struct ActivePackageTask {
    operation_label: &'static str,
    manager: PackageManagerType,
    package_name: String,
    state: content::PackageTaskState,
    progress: f32,
}

#[derive(Debug, Clone)]
pub struct App {
    pub sidebar: SideBar,
    pub content: Content,

    pub pm_config: updater_core::Config,
    pub installed_info: InstalledInfo,
    pub updates_info: UpdatesInfo,
    pub finding_info: FindingInfo,

    progress_animation: Animation<f32>,
    progress_target: f32,
    is_indeterminate_progress: bool,
    animation_origin: Instant,
    last_frame: Instant,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let now = Instant::now();

        let app = Self {
            sidebar: SideBar::default(),
            content: Content::default(),
            pm_config: updater_core::Config::default(),
            installed_info: InstalledInfo::default(),
            updates_info: UpdatesInfo::default(),
            finding_info: FindingInfo::default(),
            progress_animation: Animation::new(0.0).duration(Duration::from_millis(280)),
            progress_target: 0.0,
            is_indeterminate_progress: false,
            animation_origin: now,
            last_frame: now,
        };

        let task = Task::perform(updater_core::Config::load(), Message::ConfigLoaded);

        (app, task)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.has_active_work()
            || self.is_indeterminate_progress
            || self.progress_animation.is_animating(self.last_frame)
        {
            iced::window::frames().map(Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn has_active_work(&self) -> bool {
        self.installed_info.is_loading_count
            || self.updates_info.is_loading_count
            || !self.installed_info.loading_installed.is_empty()
            || !self.updates_info.loading_updates.is_empty()
            || !self.finding_info.searching_managers.is_empty()
            || self.finding_info.is_installing
            || self.updates_info.is_updating
            || self.installed_info.is_removing
    }

    fn progress_mode(&self) -> ProgressMode {
        let mut known_total = 0usize;
        let mut known_done = 0usize;

        if !self.finding_info.searching_managers.is_empty() {
            let total = self.finding_info.selected_managers.len();
            let searching = self.finding_info.searching_managers.len();
            known_total += total;
            known_done += total.saturating_sub(searching);
        }

        if !self.installed_info.loading_installed.is_empty() {
            let total = self.installed_info.selected_managers.len();
            let loading = self.installed_info.loading_installed.len();
            known_total += total;
            known_done += total.saturating_sub(loading);
        }

        if !self.updates_info.loading_updates.is_empty() {
            let total = self.updates_info.selected_managers.len();
            let loading = self.updates_info.loading_updates.len();
            known_total += total;
            known_done += total.saturating_sub(loading);
        }

        if self.finding_info.is_installing
            && let Some((completed, total, _, _)) = &self.finding_info.install_progress
            && *total > 0
        {
            known_total += *total;
            known_done += (*completed).min(*total);
        }

        if self.updates_info.is_updating
            && let Some((completed, total, _, _)) = &self.updates_info.update_progress
            && *total > 0
        {
            known_total += *total;
            known_done += (*completed).min(*total);
        }

        if self.installed_info.is_removing
            && let Some((completed, total, _, _)) = &self.installed_info.remove_progress
            && *total > 0
        {
            known_total += *total;
            known_done += (*completed).min(*total);
        }

        if known_total > 0 {
            ProgressMode::Determinate((known_done as f32 / known_total as f32).clamp(0.0, 1.0))
        } else if self.has_active_work() {
            ProgressMode::Indeterminate
        } else {
            ProgressMode::Determinate(1.0)
        }
    }

    fn sync_progress_animation(&mut self, at: Instant) {
        match self.progress_mode() {
            ProgressMode::Determinate(target) => {
                if self.is_indeterminate_progress {
                    self.is_indeterminate_progress = false;
                }

                if (self.progress_target - target).abs() > 0.001 {
                    self.progress_target = target;
                    self.progress_animation.go_mut(target, at);
                }
            }
            ProgressMode::Indeterminate => {
                if !self.is_indeterminate_progress {
                    self.is_indeterminate_progress = true;
                    self.animation_origin = at;
                }
            }
        }
    }

    fn current_progress_value(&self) -> f32 {
        self.progress_animation
            .interpolate_with(|value| value, self.last_frame)
            .clamp(0.0, 1.0)
    }

    fn moving_phase(&self, cycle: Duration) -> f32 {
        let cycle_ms = cycle.as_millis().max(1);
        let elapsed_ms = self
            .last_frame
            .saturating_duration_since(self.animation_origin)
            .as_millis();

        (elapsed_ms % cycle_ms) as f32 / cycle_ms as f32
    }

    fn active_package_tasks(&self) -> Vec<ActivePackageTask> {
        let mut tasks = Vec::new();

        if self.finding_info.is_installing {
            tasks.extend(
                self.finding_info
                    .install_items
                    .iter()
                    .map(|item| ActivePackageTask {
                        operation_label: "Install",
                        manager: item.manager,
                        package_name: item.package_name.clone(),
                        state: item.state,
                        progress: item.progress,
                    }),
            );
        }

        if self.updates_info.is_updating {
            tasks.extend(
                self.updates_info
                    .update_items
                    .iter()
                    .map(|item| ActivePackageTask {
                        operation_label: "Update",
                        manager: item.manager,
                        package_name: item.package_name.clone(),
                        state: item.state,
                        progress: item.progress,
                    }),
            );
        }

        if self.installed_info.is_removing {
            tasks.extend(
                self.installed_info
                    .remove_items
                    .iter()
                    .map(|item| ActivePackageTask {
                        operation_label: "Remove",
                        manager: item.manager,
                        package_name: item.package_name.clone(),
                        state: item.state,
                        progress: item.progress,
                    }),
            );
        }

        tasks
    }

    fn current_status_label(&self) -> String {
        if self.installed_info.is_loading_count || self.updates_info.is_loading_count {
            return "Initializing package manager data...".to_string();
        }

        if self.finding_info.is_installing {
            if let Some((completed, total, manager, package)) = &self.finding_info.install_progress
            {
                if package.is_empty() {
                    return format!("Installing packages ({}/{})...", completed, total);
                }

                return format!(
                    "Installing {}/{}: {} ({})",
                    completed,
                    total,
                    package,
                    manager.name()
                );
            }

            return "Installing selected packages...".to_string();
        }

        if self.updates_info.is_updating {
            if let Some((completed, total, manager, package)) = &self.updates_info.update_progress {
                if package.is_empty() {
                    return format!("Updating packages ({}/{})...", completed, total);
                }

                return format!(
                    "Updating {}/{}: {} ({})",
                    completed,
                    total,
                    package,
                    manager.name()
                );
            }

            return "Updating selected packages...".to_string();
        }

        if self.installed_info.is_removing {
            if let Some((completed, total, manager, package)) = &self.installed_info.remove_progress
            {
                if package.is_empty() {
                    return format!("Removing packages ({}/{})...", completed, total);
                }

                return format!(
                    "Removing {}/{}: {} ({})",
                    completed,
                    total,
                    package,
                    manager.name()
                );
            }

            return "Removing selected packages...".to_string();
        }

        if !self.finding_info.searching_managers.is_empty() {
            let total = self.finding_info.selected_managers.len();
            let searching = self.finding_info.searching_managers.len();
            let done = total.saturating_sub(searching);
            return format!("Searching packages ({}/{})...", done, total);
        }

        if !self.installed_info.loading_installed.is_empty() {
            let total = self.installed_info.selected_managers.len();
            let loading = self.installed_info.loading_installed.len();
            let done = total.saturating_sub(loading);
            return format!("Loading installed packages ({}/{})...", done, total);
        }

        if !self.updates_info.loading_updates.is_empty() {
            let total = self.updates_info.selected_managers.len();
            let loading = self.updates_info.loading_updates.len();
            let done = total.saturating_sub(loading);
            return format!("Loading updates ({}/{})...", done, total);
        }

        "Idle".to_string()
    }

    fn indeterminate_activity_strip(&self, phase: f32) -> iced::Element<'_, Message> {
        use iced::{
            Border,
            widget::{container, row},
        };

        let segment_count = 36usize;
        let highlight_width = 10usize;
        let head =
            ((phase.clamp(0.0, 1.0) * segment_count as f32).floor() as usize) % segment_count;

        let strip = row((0..segment_count).map(|index| {
            let distance = (index + segment_count - head) % segment_count;
            let alpha = if distance < highlight_width {
                let t = 1.0 - (distance as f32 / highlight_width as f32);
                0.20 + (t * 0.80)
            } else {
                0.0
            };

            let color = if alpha > 0.0 {
                iced::Color::from_rgba(
                    colors::SECONDARY_SOFT.r,
                    colors::SECONDARY_SOFT.g,
                    colors::SECONDARY_SOFT.b,
                    alpha,
                )
            } else {
                colors::DIVIDER_LIGHT
            };

            container("")
                .width(Length::FillPortion(1))
                .height(Length::Fixed(8.0))
                .style(move |_theme: &iced::Theme| container::Style {
                    background: Some(color.into()),
                    border: Border {
                        color: iced::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 999.0.into(),
                    },
                    text_color: None,
                    shadow: Default::default(),
                    snap: false,
                })
                .into()
        }))
        .spacing(2)
        .width(Length::Fill)
        .align_y(iced::Alignment::Center);

        container(strip)
            .padding([4, 6])
            .width(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(colors::SURFACE_HOVER.into()),
                border: Border {
                    color: colors::DIVIDER_LIGHT,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                text_color: None,
                shadow: Default::default(),
                snap: false,
            })
            .into()
    }

    fn determinate_capsule_bar(&self, progress: f32) -> iced::Element<'_, Message> {
        use iced::{
            Border,
            widget::{Space, container, row},
        };

        let travel_portion: u16 = 1000;
        let filled =
            ((progress.clamp(0.0, 1.0) * travel_portion as f32).round() as u16).min(travel_portion);
        let remaining = travel_portion.saturating_sub(filled);

        let capsule = container("")
            .width(if filled == 0 {
                Length::Shrink
            } else {
                Length::FillPortion(filled)
            })
            .height(Length::Fixed(8.0))
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(colors::SECONDARY_SOFT.into()),
                border: Border {
                    color: iced::Color::from_rgba(
                        colors::SECONDARY_SOFT.r,
                        colors::SECONDARY_SOFT.g,
                        colors::SECONDARY_SOFT.b,
                        0.6,
                    ),
                    width: 1.0,
                    radius: 999.0.into(),
                },
                text_color: None,
                shadow: iced::Shadow {
                    color: iced::Color::from_rgba(
                        colors::SECONDARY_SOFT.r,
                        colors::SECONDARY_SOFT.g,
                        colors::SECONDARY_SOFT.b,
                        0.28,
                    ),
                    offset: iced::Vector::new(0.0, 0.0),
                    blur_radius: 8.0,
                },
                snap: false,
            });

        let right_spacer = Space::new().width(if remaining == 0 {
            Length::Shrink
        } else {
            Length::FillPortion(remaining)
        });

        let bar = row![capsule, right_spacer]
            .width(Length::Fill)
            .align_y(iced::Alignment::Center);

        container(bar)
            .padding([4, 6])
            .width(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(colors::SURFACE_HOVER.into()),
                border: Border {
                    color: colors::DIVIDER_LIGHT,
                    width: 1.0,
                    radius: 999.0.into(),
                },
                text_color: None,
                shadow: Default::default(),
                snap: false,
            })
            .into()
    }

    fn package_task_row(&self, task: ActivePackageTask) -> iced::Element<'_, Message> {
        use iced::{
            Border,
            widget::{column, container, row, text},
        };

        let (state_text, state_color) = match task.state {
            content::PackageTaskState::Pending => ("Pending".to_string(), colors::ON_SURFACE_MUTED),
            content::PackageTaskState::InProgress => {
                if task.progress > 0.0 {
                    (
                        format!("Running {:.0}%", (task.progress * 100.0).clamp(0.0, 100.0)),
                        colors::SECONDARY,
                    )
                } else {
                    ("Running".to_string(), colors::SECONDARY)
                }
            }
            content::PackageTaskState::Done => ("Done".to_string(), colors::SUCCESS),
        };

        let progress_widget = match task.state {
            content::PackageTaskState::Pending => self.determinate_capsule_bar(0.0),
            content::PackageTaskState::InProgress => {
                if task.progress > 0.0 {
                    self.determinate_capsule_bar(task.progress)
                } else {
                    self.indeterminate_activity_strip(
                        self.moving_phase(Duration::from_millis(2200)),
                    )
                }
            }
            content::PackageTaskState::Done => self.determinate_capsule_bar(1.0),
        };

        container(
            column![
                row![
                    text(format!(
                        "{} · {} ({})",
                        task.operation_label,
                        task.package_name,
                        task.manager.name()
                    ))
                    .size(13)
                    .color(colors::ON_SURFACE)
                    .width(Length::Fill),
                    text(state_text).size(12).color(state_color)
                ]
                .align_y(iced::Alignment::Center)
                .spacing(10),
                progress_widget
            ]
            .spacing(6),
        )
        .padding([8, 10])
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(colors::SURFACE.into()),
            border: Border {
                color: colors::DIVIDER_LIGHT,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: None,
            shadow: Default::default(),
            snap: false,
        })
        .into()
    }

    fn status_panel(&self) -> iced::Element<'_, Message> {
        use iced::{
            Border,
            widget::{column, container, row, scrollable, text},
        };

        let progress = self.current_progress_value();
        let indeterminate_phase = self.moving_phase(Duration::from_millis(2200));
        let status_label = self.current_status_label();
        let status_right = if self.is_indeterminate_progress {
            "Indeterminate".to_string()
        } else {
            format!("{:.0}%", progress * 100.0)
        };

        let progress_widget: iced::Element<'_, Message> = if self.is_indeterminate_progress {
            self.indeterminate_activity_strip(indeterminate_phase)
        } else {
            self.determinate_capsule_bar(progress)
        };

        let active_tasks = self.active_package_tasks();
        let active_task_count = active_tasks.len();

        let mut panel_content = column![
            row![
                text(status_label).size(14).color(colors::ON_SURFACE),
                text(status_right).size(13).color(colors::ON_SURFACE_MUTED)
            ]
            .align_y(iced::Alignment::Center)
            .spacing(12),
            progress_widget
        ]
        .spacing(10)
        .height(Length::Fill);

        if !active_tasks.is_empty() {
            let task_rows: Vec<iced::Element<'_, Message>> = active_tasks
                .into_iter()
                .map(|task| self.package_task_row(task))
                .collect();

            let tasks_list = scrollable(column(task_rows).spacing(8))
                .height(Length::Fill)
                .width(Length::Fill);

            panel_content = panel_content
                .push(
                    text(format!("Package Progress ({})", active_task_count))
                        .size(12)
                        .color(colors::ON_SURFACE_MUTED),
                )
                .push(tasks_list);
        }

        let panel_height = if active_task_count == 0 { 86.0 } else { 280.0 };

        container(panel_content)
            .padding([10, 16])
            .height(Length::Fixed(panel_height))
            .width(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(colors::SURFACE_ALT.into()),
                border: Border {
                    color: colors::DIVIDER,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                text_color: None,
                shadow: Default::default(),
                snap: false,
            })
            .into()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SideBar(sidebar::Message),
    Content(content::Message),
    ConfigLoaded(Result<updater_core::Config, updater_core::error::CoreError>),
    InitInstalledCounts(Vec<(PackageManagerType, usize)>),
    InitUpdatesCounts(Vec<(PackageManagerType, Vec<PackageUpdate>)>),
    Tick(Instant),
}

impl App {
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let at = match &message {
            Message::Tick(at) => *at,
            _ => Instant::now(),
        };

        self.last_frame = at;

        let mut task = Task::none();

        match message {
            Message::Tick(_) => {}
            Message::SideBar(sidebar_msg) => match self.sidebar.update(sidebar_msg) {
                sidebar::Action::ChangeContent(content) => {
                    self.content.active_content = content;
                }
                sidebar::Action::Run(sidebar_task) => {
                    task = sidebar_task.map(Message::SideBar);
                }
                sidebar::Action::None => {}
            },
            Message::Content(content_msg) => {
                let action = self.content.update(
                    content_msg,
                    &mut self.pm_config,
                    &mut self.installed_info,
                    &mut self.updates_info,
                    &mut self.finding_info,
                );

                match action {
                    content::Action::Run(content_task) => {
                        task = content_task.map(Message::Content);
                    }
                    content::Action::ReloadInstalledData => {
                        self.installed_info.is_loading_count = true;
                        task = Task::future(Self::init_installed_counts(self.pm_config.clone()))
                            .then(|installed_counts| {
                                Task::done(Message::InitInstalledCounts(installed_counts))
                            });
                    }
                    content::Action::None => {}
                }
            }
            Message::ConfigLoaded(result) => match result {
                Ok(config) => {
                    self.pm_config = config;
                    self.installed_info.is_loading_count = true;
                    self.updates_info.is_loading_count = true;

                    let installed_task = Task::future(Self::init_installed_counts(
                        self.pm_config.clone(),
                    ))
                    .then(|installed_counts| {
                        Task::done(Message::InitInstalledCounts(installed_counts))
                    });

                    let updates_task = Task::future(Self::init_updates_counts(
                        self.pm_config.clone(),
                    ))
                    .then(|update_counts| Task::done(Message::InitUpdatesCounts(update_counts)));

                    task = Task::batch(vec![installed_task, updates_task]);
                }
                Err(e) => {
                    log::error!("Failed to load config: {}", e);
                }
            },
            Message::InitInstalledCounts(counts) => {
                self.installed_info.is_loading_count = false;
                self.installed_info.has_loading_count = true;

                self.installed_info.installed_packages = counts
                    .into_iter()
                    .map(|(pm_type, count)| (pm_type, (count, Vec::new())))
                    .collect();
            }
            Message::InitUpdatesCounts(updates) => {
                self.updates_info.is_loading_count = false;
                self.updates_info.has_loading_count = true;

                self.updates_info.updates_by_manager = updates
                    .into_iter()
                    .map(|(pm_type, packages)| {
                        let count = packages.len();
                        (pm_type, (count, packages))
                    })
                    .collect();
            }
        }

        self.sync_progress_animation(at);
        task
    }

    pub fn view(&self) -> iced::Element<'_, Message> {
        use iced::{
            Border, Shadow, Vector,
            widget::{column, container, row},
        };

        let sidebar = container(self.sidebar.view().map(Message::SideBar))
            .padding(16)
            .width(Length::Fixed(220.0))
            .height(Length::Fill)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(colors::SURFACE_ALT.into()),
                border: Border {
                    color: colors::DIVIDER_LIGHT,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                text_color: None,
                shadow: Shadow {
                    color: colors::SHADOW_LIGHT,
                    offset: Vector::new(2.0, 0.0),
                    blur_radius: 8.0,
                },
                snap: false,
            });

        let content_area = container(
            self.content
                .view(
                    &self.pm_config,
                    &self.installed_info,
                    &self.updates_info,
                    &self.finding_info,
                )
                .map(Message::Content),
        )
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(iced::Color::WHITE.into()),
            border: Border::default(),
            text_color: None,
            shadow: Shadow::default(),
            snap: false,
        });

        let top_layout = container(
            row![sidebar, content_area]
                .spacing(0)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_theme: &iced::Theme| container::Style {
            background: Some(colors::SURFACE_MUTED.into()),
            border: Border::default(),
            text_color: None,
            shadow: Shadow::default(),
            snap: false,
        });

        column![top_layout, self.status_panel()]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    async fn init_installed_counts(
        config: updater_core::Config,
    ) -> Vec<(PackageManagerType, usize)> {
        let config = Arc::new(config);

        let pms: Vec<PackageManagerType> = config
            .system_manager
            .iter()
            .map(|pm| pm.manager_type)
            .chain(config.app_managers.iter().map(|pm| pm.manager_type))
            .collect();

        join_all(pms.into_iter().map(|pm| {
            let config = config.clone();
            async move {
                let count = pm.count_installed(&config).await.unwrap_or(0);
                (pm, count)
            }
        }))
        .await
    }

    async fn init_updates_counts(
        config: updater_core::Config,
    ) -> Vec<(PackageManagerType, Vec<PackageUpdate>)> {
        let config = Arc::new(config);

        let pms: Vec<PackageManagerType> = config
            .system_manager
            .iter()
            .map(|pm| pm.manager_type)
            .chain(config.app_managers.iter().map(|pm| pm.manager_type))
            .collect();

        join_all(pms.into_iter().map(|pm| {
            let config = config.clone();
            async move {
                let updates = pm.list_updates(&config).await.unwrap_or_default();
                (pm, updates)
            }
        }))
        .await
    }
}
