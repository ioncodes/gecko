use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_wgpu::capture;
use iced::theme::Mode;
use iced::widget::{Stack, button, column, container, mouse_area, scrollable, stack, text};
use iced::{Background, Border, Color, Element, Length, Padding, Subscription, Task, Theme, window};

use crate::cache::{self, LibraryCache};
use crate::config::{self, Config};
use crate::game::{CpuMode, Game, Platform, ThemePreference};
use crate::library::{self, ScanProgress};
use crate::player::{self, PlayerState, PlayerStatus};
use crate::theme::{self, Palette};
use crate::widgets::{game_row, menubar, search_bar, statusbar};

const REPO_URL: &str = "https://github.com/ioncodes/gecko";
const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);
const TOAST_DURATION: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub enum Message {
    Noop,
    LibraryWindowOpened(window::Id),
    PlayerWindowOpened(window::Id, Box<Game>, Arc<PlayerState>),
    PlayerTick,
    WindowClosed(window::Id),
    LibraryPicked(Platform, Option<PathBuf>),
    ScanRequested,
    ScanProgress(ScanProgress),
    SearchChanged(String),
    MenuChooseLibrary(Platform),
    MenuRescan,
    MenuQuit,
    MenuToggleCpu(CpuMode),
    MenuToggleSkipIpl,
    MenuSetTheme(ThemePreference),
    MenuAbout,
    AboutClose,
    OpenRepo,
    GameClicked(usize),
    LaunchPlayer(usize),
    SystemThemeLoaded(Mode),
    SystemThemeChanged(Mode),
    PlayerToggleFullscreen(window::Id),
    PlayerToggleOverlay(window::Id),
    PlayerToggleUncapped(window::Id),
    PlayerScreenshot(window::Id),
}

pub struct Toast {
    title: String,
    detail: String,
    expires: Instant,
}

impl Toast {
    fn new(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            expires: Instant::now() + TOAST_DURATION,
        }
    }
}

pub struct PlayerWindow {
    pub(crate) game: Game,
    pub(crate) state: Arc<PlayerState>,
    pub(crate) fullscreen: bool,
    pub(crate) overlay: bool,
    pub(crate) toast: Option<Toast>,
}

pub struct App {
    cache: LibraryCache,
    config: Config,
    cli_gcn_override: Option<PathBuf>,
    cli_wii_override: Option<PathBuf>,
    games: Vec<Game>,
    search: String,
    search_lc: String,
    scanning: bool,
    about_open: bool,
    system_mode: Mode,
    palette: Palette,
    last_click: Option<(usize, Instant)>,
    library_window: Option<window::Id>,
    players: HashMap<window::Id, PlayerWindow>,
}

impl App {
    pub fn boot(cli_gcn: Option<PathBuf>, cli_wii: Option<PathBuf>) -> (Self, Task<Message>) {
        let config = config::load(&config::config_path());
        let cache = cache::load(&cache::cache_path());

        let mut games: Vec<Game> = cache.entries.values().map(|e| e.game.clone()).collect();
        games.sort_by(|a, b| a.title_lc.cmp(&b.title_lc));

        let has_any_root =
            cli_gcn.is_some() || cli_wii.is_some() || config.gcn_library.is_some() || config.wii_library.is_some();

        let palette = self::resolve_palette(config.theme, Mode::Light);
        let app = Self {
            cache,
            config,
            cli_gcn_override: cli_gcn,
            cli_wii_override: cli_wii,
            games,
            search: String::new(),
            search_lc: String::new(),
            scanning: false,
            about_open: false,
            system_mode: Mode::Light,
            palette,
            last_click: None,
            library_window: None,
            players: HashMap::new(),
        };

        let (_, open_lib) = window::open(window::Settings {
            size: iced::Size::new(900.0, 720.0),
            ..window::Settings::default()
        });

        let mut tasks: Vec<Task<Message>> = Vec::new();
        tasks.push(open_lib.map(Message::LibraryWindowOpened));
        tasks.push(iced::system::theme().map(Message::SystemThemeLoaded));
        if has_any_root {
            tasks.push(Task::done(Message::ScanRequested));
        }

        (app, Task::batch(tasks))
    }

    pub fn title(&self, window_id: window::Id) -> String {
        if Some(window_id) == self.library_window {
            "Gecko".to_owned()
        } else if let Some(player) = self.players.get(&window_id) {
            format!("Gecko — {}", player.game.title)
        } else {
            "Gecko".to_owned()
        }
    }

    pub fn theme(&self, _window_id: window::Id) -> Theme {
        theme::theme(&self.palette)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::system::theme_changes().map(Message::SystemThemeChanged),
            window::close_events().map(Message::WindowClosed),
        ];

        if !self.players.is_empty() {
            subs.push(window::frames().map(|_| Message::PlayerTick));
        }

        Subscription::batch(subs)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Noop => Task::none(),
            Message::LibraryWindowOpened(id) => {
                self.library_window = Some(id);
                Task::none()
            }
            Message::PlayerWindowOpened(id, game, state) => {
                tracing::info!(window = ?id, title = %game.title, "player window opened");
                self.players.insert(
                    id,
                    PlayerWindow {
                        game: *game,
                        state,
                        fullscreen: false,
                        overlay: false,
                        toast: None,
                    },
                );
                Task::none()
            }
            Message::PlayerTick => {
                let now = Instant::now();
                for player in self.players.values_mut() {
                    if player.toast.as_ref().is_some_and(|t| now >= t.expires) {
                        player.toast = None;
                    }
                }
                Task::none()
            }
            Message::WindowClosed(id) => {
                if Some(id) == self.library_window {
                    self.library_window = None;
                    iced::exit()
                } else if let Some(player) = self.players.remove(&id) {
                    tracing::info!(window = ?id, "player window closed");
                    player.state.shutdown();
                    Task::none()
                } else {
                    Task::none()
                }
            }
            Message::LibraryPicked(_, None) => Task::none(),
            Message::LibraryPicked(platform, Some(path)) => {
                match platform {
                    Platform::Gcn => self.config.gcn_library = Some(path),
                    Platform::Wii => self.config.wii_library = Some(path),
                }
                self.persist_config();
                Task::done(Message::ScanRequested)
            }
            Message::ScanRequested => {
                let roots = self.effective_library_roots();
                if roots.is_empty() {
                    return Task::none();
                }

                self.scanning = true;
                let prior = self.cache.clone();
                Task::stream(library::scan_library_stream(roots, prior)).map(Message::ScanProgress)
            }
            Message::ScanProgress(ScanProgress::Started { cached, pending }) => {
                self.games = cached;
                self.games.sort_by(|a, b| a.title_lc.cmp(&b.title_lc));
                tracing::info!(cached = self.games.len(), pending, "scan started");
                Task::none()
            }
            Message::ScanProgress(ScanProgress::Loaded(game)) => {
                self.insert_game_sorted(*game);
                Task::none()
            }
            Message::ScanProgress(ScanProgress::Finished(cache)) => {
                self.cache = *cache;
                self.persist_cache();
                self.scanning = false;
                tracing::info!(total = self.games.len(), "scan finished");
                Task::none()
            }
            Message::ScanProgress(ScanProgress::Error(err)) => {
                tracing::warn!(%err, "scan failed");
                self.scanning = false;
                Task::none()
            }
            Message::SearchChanged(q) => {
                self.search_lc = q.to_lowercase();
                self.search = q;
                Task::none()
            }
            Message::MenuChooseLibrary(platform) => {
                let title = match platform {
                    Platform::Gcn => "Select GameCube folder",
                    Platform::Wii => "Select Wii folder",
                };
                Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title(title)
                            .pick_folder()
                            .await
                            .map(|h| h.path().to_path_buf())
                    },
                    move |opt| Message::LibraryPicked(platform, opt),
                )
            }
            Message::MenuRescan => {
                if self.effective_library_roots().is_empty() {
                    Task::done(Message::MenuChooseLibrary(Platform::Gcn))
                } else {
                    Task::done(Message::ScanRequested)
                }
            }
            Message::MenuQuit => iced::exit(),
            Message::MenuToggleCpu(mode) => {
                self.config.cpu_mode = mode;
                self.persist_config();
                Task::none()
            }
            Message::MenuToggleSkipIpl => {
                self.config.skip_ipl = !self.config.skip_ipl;
                self.persist_config();
                Task::none()
            }
            Message::MenuSetTheme(pref) => {
                self.config.theme = pref;
                self.persist_config();
                self.refresh_palette();
                Task::none()
            }
            Message::MenuAbout => {
                self.about_open = true;
                Task::none()
            }
            Message::AboutClose => {
                self.about_open = false;
                Task::none()
            }
            Message::OpenRepo => {
                if let Err(err) = webbrowser::open(REPO_URL) {
                    tracing::warn!(%err, url = REPO_URL, "failed to open repo URL");
                }
                Task::none()
            }
            Message::GameClicked(idx) => {
                let now = Instant::now();
                let is_double = self
                    .last_click
                    .is_some_and(|(prev, when)| prev == idx && now.duration_since(when) <= DOUBLE_CLICK_WINDOW);

                if is_double {
                    self.last_click = None;
                    Task::done(Message::LaunchPlayer(idx))
                } else {
                    self.last_click = Some((idx, now));
                    if let Some(game) = self.games.get(idx) {
                        tracing::info!(path = %game.path.display(), title = %game.title, "game selected");
                    }
                    Task::none()
                }
            }
            Message::LaunchPlayer(idx) => {
                let Some(game) = self.games.get(idx).cloned() else {
                    return Task::none();
                };

                let state = PlayerState::new(&game, &self.config);
                let (_, open) = window::open(window::Settings {
                    size: iced::Size::new(960.0, 720.0),
                    ..window::Settings::default()
                });

                open.map(move |id| Message::PlayerWindowOpened(id, Box::new(game.clone()), state.clone()))
            }
            Message::SystemThemeLoaded(mode) | Message::SystemThemeChanged(mode) => {
                self.system_mode = mode;
                self.refresh_palette();
                Task::none()
            }
            Message::PlayerToggleFullscreen(id) => {
                let Some(player) = self.players.get_mut(&id) else {
                    return Task::none();
                };

                player.fullscreen = !player.fullscreen;
                let mode = if player.fullscreen {
                    window::Mode::Fullscreen
                } else {
                    window::Mode::Windowed
                };
                window::set_mode(id, mode)
            }
            Message::PlayerToggleOverlay(id) => {
                if let Some(player) = self.players.get_mut(&id) {
                    player.overlay = !player.overlay;
                }
                Task::none()
            }
            Message::PlayerToggleUncapped(id) => {
                let Some(player) = self.players.get_mut(&id) else {
                    return Task::none();
                };

                let (title, detail) = if player.state.toggle_throttle() {
                    ("Speed limit on", "Throttled to native speed")
                } else {
                    ("Speed limit off", "Running uncapped")
                };

                player.toast = Some(Toast::new(title, detail));
                Task::none()
            }
            Message::PlayerScreenshot(id) => {
                let Some(player) = self.players.get_mut(&id) else {
                    return Task::none();
                };

                let (title, detail) = match player.state.capture_frame() {
                    Some(frame) => {
                        let path = self::screenshot_path(&player.game.game_id);
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        capture::save_png_async(path, frame, true);
                        ("Screenshot saved", name)
                    }
                    None => ("Screenshot failed", "No frame to capture yet".to_owned()),
                };

                player.toast = Some(Toast::new(title, detail));
                Task::none()
            }
        }
    }

    pub fn view(&self, window_id: window::Id) -> Element<'_, Message> {
        if Some(window_id) == self.library_window {
            self.library_view()
        } else if let Some(player) = self.players.get(&window_id) {
            self.player_view(window_id, player)
        } else {
            container(text("")).width(Length::Fill).height(Length::Fill).into()
        }
    }

    fn library_view(&self) -> Element<'_, Message> {
        let palette = &self.palette;

        let body: Element<'_, Message> = if self.games.is_empty() {
            let (msg, hint) = if self.scanning {
                ("Scanning…", "Reading disc headers")
            } else if self.effective_library_roots().is_empty() {
                ("No libraries set", "File → Set GameCube / Wii Folder…")
            } else {
                ("No games found", "Drop ISO, RVZ, or ZIP files into the library folder")
            };
            game_row::empty_state(palette, msg, hint)
        } else {
            let mut list = column![].spacing(0);
            for (idx, game) in self.games.iter().enumerate() {
                if !game.matches_lc(&self.search_lc) {
                    continue;
                }
                list = list.push(game_row::game_row(palette, idx, game));
            }
            scrollable(list).height(Length::Fill).width(Length::Fill).into()
        };

        let bg = palette.bg;
        let text_color = palette.text;
        let main = column![
            menubar::menubar(palette, self.config.cpu_mode, self.config.theme, self.config.skip_ipl),
            search_bar::search_bar(palette, &self.search),
            body,
            statusbar::statusbar(palette, self.config.cpu_mode, self.games.len(), self.scanning),
        ];

        let root = container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(bg)),
                text_color: Some(text_color),
                ..container::Style::default()
            });

        let root_element: Element<'_, Message> = root.into();
        if self.about_open {
            stack![root_element, self::about_overlay(palette)].into()
        } else {
            root_element
        }
    }

    fn player_view<'a>(&'a self, window_id: window::Id, player: &'a PlayerWindow) -> Element<'a, Message> {
        let palette = &self.palette;
        let bg = palette.bg;
        let text_color = palette.text;

        let status = player.state.status();
        let shader: Element<'a, Message> = player::shader_widget(player.state.clone(), window_id).into();

        let overlay: Option<Element<'a, Message>> = match &status {
            PlayerStatus::Ready => None,
            PlayerStatus::Booting => Some(self::overlay_card(
                palette,
                &format!("Loading {}", player.game.title),
                Some("Decompressing disc and warming up CPU heatsink..."),
                false,
            )),
            PlayerStatus::Failed(msg) => Some(self::overlay_card(palette, "Cannot start game", Some(msg), true)),
        };

        let mut layers: Vec<Element<'a, Message>> = vec![shader];
        if let Some(ov) = overlay {
            layers.push(ov);
        }
        if player.overlay {
            let (fps, native_pct) = player.state.fps();
            layers.push(self::stats_overlay(palette, fps, native_pct));
        }
        if let Some(toast) = &player.toast {
            layers.push(self::toast_overlay(palette, toast));
        }

        let body: Element<'a, Message> = Stack::with_children(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();

        container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(bg)),
                text_color: Some(text_color),
                ..container::Style::default()
            })
            .into()
    }

    fn effective_library_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(p) = self
            .cli_gcn_override
            .clone()
            .or_else(|| self.config.gcn_library.clone())
        {
            roots.push(p);
        }

        if let Some(p) = self
            .cli_wii_override
            .clone()
            .or_else(|| self.config.wii_library.clone())
        {
            roots.push(p);
        }

        roots
    }

    fn persist_config(&self) {
        let path = config::config_path();
        if let Err(err) = config::save(&path, &self.config) {
            tracing::warn!(%err, path = %path.display(), "failed to persist config");
        }
    }

    fn persist_cache(&self) {
        let path = cache::cache_path();
        if let Err(err) = cache::save(&path, &self.cache) {
            tracing::warn!(%err, path = %path.display(), "failed to persist library cache");
        }
    }

    fn insert_game_sorted(&mut self, game: Game) {
        let pos = self
            .games
            .binary_search_by(|g| g.title_lc.cmp(&game.title_lc))
            .unwrap_or_else(|p| p);
        self.games.insert(pos, game);
    }

    fn refresh_palette(&mut self) {
        self.palette = self::resolve_palette(self.config.theme, self.system_mode);
    }
}

fn resolve_palette(pref: ThemePreference, system_mode: Mode) -> Palette {
    let effective = match pref {
        ThemePreference::Light => Mode::Light,
        ThemePreference::Dark => Mode::Dark,
        ThemePreference::System => system_mode,
    };
    Palette::for_mode(effective)
}

fn screenshot_path(game_id: &str) -> PathBuf {
    config::exe_relative(capture::timestamped_path(game_id))
}

fn corner_card<'a>(palette: &Palette, content: Element<'a, Message>) -> Element<'a, Message> {
    let card_bg = Color { a: 0.85, ..palette.bg };
    let border_2 = palette.border_2;

    container(content)
        .padding(Padding::from([8, 12]))
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(card_bg)),
            border: Border {
                color: border_2,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn stats_overlay(palette: &Palette, fps: f32, native_pct: f32) -> Element<'static, Message> {
    let label = text(format!("{fps:.1} FPS  {native_pct:.0}%"))
        .size(13)
        .color(palette.text);
    let card = self::corner_card(palette, label.into());

    container(card)
        .width(Length::Fill)
        .align_x(iced::Alignment::End)
        .padding(12)
        .into()
}

fn toast_overlay<'a>(palette: &Palette, toast: &'a Toast) -> Element<'a, Message> {
    let content = column![
        text(&toast.title).size(13).color(palette.text),
        text(&toast.detail).size(11).color(palette.text_dim),
    ]
    .spacing(2);
    let card = self::corner_card(palette, content.into());

    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::Alignment::End)
        .align_y(iced::Alignment::End)
        .padding(12)
        .into()
}

fn overlay_card(palette: &Palette, title: &str, detail: Option<&str>, error: bool) -> Element<'static, Message> {
    let bg = palette.bg;
    let border_2 = palette.border_2;
    let text_color = palette.text;
    let dim = palette.text_dim;
    let accent = if error { palette.purple } else { palette.accent };

    let backdrop_color = Color {
        a: 0.55,
        ..(if palette.is_dark { Color::BLACK } else { palette.text })
    };

    let mut card_col = column![text(title.to_owned()).size(20).color(text_color)].spacing(8);
    card_col = card_col.push(
        container(text(""))
            .width(40)
            .height(2)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(accent)),
                ..container::Style::default()
            }),
    );
    if let Some(detail) = detail {
        card_col = card_col.push(text(detail.to_owned()).size(12).color(dim));
    }

    let card: Element<'static, Message> = container(card_col.width(Length::Fill).align_x(iced::Alignment::Center))
        .width(Length::Fixed(420.0))
        .padding(24)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                color: border_2,
                width: 1.0,
                radius: 12.0.into(),
            },
            ..container::Style::default()
        })
        .into();

    let backdrop: Element<'static, Message> = container(text(""))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(backdrop_color)),
            ..container::Style::default()
        })
        .into();

    let centered: Element<'static, Message> = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();

    stack![backdrop, centered].into()
}

fn about_overlay(palette: &Palette) -> Element<'static, Message> {
    let bg = palette.bg;
    let surface = palette.surface;
    let surface_2 = palette.surface_2;
    let border = palette.border;
    let border_2 = palette.border_2;
    let text_color = palette.text;
    let dim = palette.text_dim;
    let mute = palette.text_mute;
    let link_color = palette.accent;
    let backdrop_color = Color {
        a: 0.45,
        ..(if palette.is_dark { Color::BLACK } else { palette.text })
    };

    let link_button = button(text(REPO_URL).size(12).color(link_color))
        .on_press(Message::OpenRepo)
        .padding(Padding::from([2, 4]))
        .style(move |_: &Theme, status| {
            let bg = match status {
                button::Status::Hovered | button::Status::Pressed => Color { a: 0.08, ..link_color },
                _ => Color::TRANSPARENT,
            };
            button::Style {
                background: Some(Background::Color(bg)),
                text_color: link_color,
                border: Border {
                    radius: 4.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        });

    let card: Element<'static, Message> = container(
        column![
            text("Gecko").size(28).color(text_color),
            text("GameCube / Wii emulator").size(13).color(dim),
            link_button,
            container(text(""))
                .width(Length::Fill)
                .height(1)
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(border)),
                    ..container::Style::default()
                }),
            column![
                text("Author").size(13).color(text_color),
                text("Layle").size(12).color(dim),
            ]
            .spacing(2)
            .align_x(iced::Alignment::Center),
            column![
                text("Acknowledgements").size(13).color(text_color),
                text("zayd").size(12).color(dim),
                text("vxpm").size(12).color(dim),
                text("hazelwiss").size(12).color(dim),
                text("Dolphin team").size(12).color(dim),
            ]
            .spacing(2)
            .align_x(iced::Alignment::Center),
            text("v0.1.0").size(11).color(mute),
            button(text("Close").size(13))
                .on_press(Message::AboutClose)
                .padding(Padding::from([6, 14]))
                .style(move |_: &Theme, status| {
                    let button_bg = match status {
                        button::Status::Hovered | button::Status::Pressed => surface_2,
                        _ => surface,
                    };
                    button::Style {
                        background: Some(Background::Color(button_bg)),
                        text_color,
                        border: Border {
                            color: border_2,
                            width: 1.0,
                            radius: 6.0.into(),
                        },
                        ..button::Style::default()
                    }
                }),
        ]
        .spacing(8)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fixed(320.0))
    .padding(24)
    .style(move |_: &Theme| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: border_2,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..container::Style::default()
    })
    .into();

    let backdrop: Element<'static, Message> = mouse_area(
        container(text(""))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(backdrop_color)),
                ..container::Style::default()
            }),
    )
    .on_press(Message::AboutClose)
    .into();

    let centered: Element<'static, Message> = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into();

    stack![backdrop, centered].into()
}
