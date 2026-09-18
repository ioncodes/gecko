use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use iced::widget::{Space, button, column, container, opaque, row, scrollable, text};
use iced::{Alignment, Background, Border, Color, Element, Length};

use crate::app::Message;
use crate::config::{self, Config, DSP_COEF_FILE, DSP_ROM_FILE, IPL_FILE};
use crate::theme::Palette;
use crate::widgets::overlay;

#[derive(Debug, Clone, Copy)]
pub enum Field {
    Dsp,
    Coef,
    Ipl,
    GameCube,
    Wii,
    Nand,
}

impl Field {
    fn info(self) -> (&'static str, &'static str) {
        match self {
            Self::Dsp => ("DSP ROM", "Required"),
            Self::Coef => ("DSP coefficients", "Required"),
            Self::Ipl => ("GameCube IPL", "GameCube only"),
            Self::GameCube => ("GameCube games", "Optional"),
            Self::Wii => ("Wii games", "Optional"),
            Self::Nand => ("Wii NAND", "Optional"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    System,
    Folders,
    Finish,
}

impl Step {
    const ALL: [Self; 3] = [Self::System, Self::Folders, Self::Finish];

    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Folders,
            _ => Self::Finish,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Finish => Self::Folders,
            _ => Self::System,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "System files",
            Self::Folders => "Game folders",
            Self::Finish => "Finish",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sources {
    dsp: Option<PathBuf>,
    coef: Option<PathBuf>,
    ipl: Option<PathBuf>,
    gcn: Option<PathBuf>,
    wii: Option<PathBuf>,
    nand: Option<PathBuf>,
}

impl Sources {
    pub fn path_mut(&mut self, field: Field) -> &mut Option<PathBuf> {
        match field {
            Field::Dsp => &mut self.dsp,
            Field::Coef => &mut self.coef,
            Field::Ipl => &mut self.ipl,
            Field::GameCube => &mut self.gcn,
            Field::Wii => &mut self.wii,
            Field::Nand => &mut self.nand,
        }
    }

    fn path(&self, field: Field) -> &Option<PathBuf> {
        match field {
            Field::Dsp => &self.dsp,
            Field::Coef => &self.coef,
            Field::Ipl => &self.ipl,
            Field::GameCube => &self.gcn,
            Field::Wii => &self.wii,
            Field::Nand => &self.nand,
        }
    }
}

pub struct Setup {
    pub sources: Sources,
    pub step: Step,
    pub busy: bool,
    pub error: Option<String>,
}

impl Setup {
    pub fn new(cfg: &Config, gcn: Option<PathBuf>, wii: Option<PathBuf>) -> Self {
        let dir = cfg.system_dir_resolved();
        let existing = |path: &Option<PathBuf>, name| Config::resolve_in_dir(path, &dir, name).filter(|p| p.is_file());

        Self {
            sources: Sources {
                dsp: existing(&cfg.dsp_rom, DSP_ROM_FILE),
                coef: existing(&cfg.dsp_coef, DSP_COEF_FILE),
                ipl: existing(&cfg.ipl, IPL_FILE),
                gcn: gcn.or_else(|| cfg.gcn_library.clone()),
                wii: wii.or_else(|| cfg.wii_library.clone()),
                nand: None,
            },
            step: Step::System,
            busy: false,
            error: None,
        }
    }

    pub fn needed(&self, cfg: &Config, config_exists: bool) -> bool {
        !cfg.setup_completed
            && (!config_exists
                || self.sources.dsp.is_none()
                || self.sources.coef.is_none()
                || (cfg.gcn_library.is_some() && self.sources.ipl.is_none()))
    }

    pub fn view(&self, palette: &Palette, cfg: &Config) -> Element<'static, Message> {
        let heading = text("Set up Gecko").size(24).font(self::semibold()).color(palette.text);

        let mut steps = row![].spacing(16);
        for (index, step) in Step::ALL.into_iter().enumerate() {
            let color = if step == self.step {
                palette.text
            } else {
                palette.text_mute
            };
            let line = if step <= self.step {
                palette.accent
            } else {
                palette.border
            };
            steps = steps.push(
                column![
                    text(format!("{}  {}", index + 1, step.label())).size(12).color(color),
                    self::divider(line),
                ]
                .spacing(10)
                .width(Length::Fill),
            );
        }

        let title = match self.step {
            Step::System => "Add system files",
            Step::Folders => "Add game folders",
            Step::Finish => "Review setup",
        };
        let mut content = column![text(title).size(17).font(self::semibold()).color(palette.text)].spacing(12);

        let fields_and_note = match self.step {
            Step::System => Some((
                [Field::Dsp, Field::Coef, Field::Ipl],
                "IPL is only needed for GameCube. Encoded IPLs are decoded automatically.",
            )),
            Step::Folders => Some((
                [Field::GameCube, Field::Wii, Field::Nand],
                "NAND is optional. Choose a root with title/ and shared2/, or leave it empty to use existing storage.",
            )),
            Step::Finish => None,
        };

        if let Some((fields, note)) = fields_and_note {
            let mut rows = column![];
            for (index, field) in fields.into_iter().enumerate() {
                if index > 0 {
                    rows = rows.push(self::divider(palette.border));
                }
                rows = rows.push(self.path_row(field, palette));
            }
            content = content.push(rows).push(self::note(note, palette));
        } else {
            let names = if self.sources.ipl.is_some() {
                format!("{DSP_ROM_FILE} · {DSP_COEF_FILE} · {IPL_FILE}")
            } else {
                format!("{DSP_ROM_FILE} · {DSP_COEF_FILE}")
            };
            content = content
                .push(self::summary_row("System files", &cfg.system_dir_resolved(), palette))
                .push(self::note(names, palette))
                .push(self::divider(palette.border));

            for (label, path) in [("GameCube games", &self.sources.gcn), ("Wii games", &self.sources.wii)] {
                if let Some(path) = path {
                    content = content.push(self::summary_row(label, path, palette));
                }
            }
            if self.sources.nand.is_some() {
                content = content.push(self::summary_row(
                    "Import Wii NAND to",
                    &gecko::paths::fs_root(),
                    palette,
                ));
            }

            let mut notes = column![].spacing(8);
            if self.sources.gcn.is_none() && self.sources.wii.is_none() {
                notes = notes.push(self::note("Add game folders later from the File menu.", palette));
            }
            if self.sources.nand.is_none() {
                notes = notes.push(self::note(
                    "Wii storage will be kept or created on first boot.",
                    palette,
                ));
            }
            notes = notes.push(self::note("System files with these names will be replaced.", palette));
            if self.sources.ipl.is_none() {
                notes = notes.push(self::note("Add a GameCube IPL later in Guided Setup.", palette));
            }
            content = content.push(
                container(column![self::divider(palette.border), notes].spacing(16))
                    .padding(iced::Padding::default().top(8)),
            );
        }

        let enabled = !self.busy;
        let mut actions = row![
            self::action("Set up later", Message::SetupClose, palette, ButtonKind::Quiet, enabled),
            Space::new().width(Length::Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if self.step > Step::System {
            actions = actions.push(self::action(
                "Back",
                Message::SetupBack,
                palette,
                ButtonKind::Secondary,
                enabled,
            ));
        }
        let (label, message) = if self.step == Step::Finish {
            ("Finish setup", Message::SetupInstall)
        } else {
            ("Continue", Message::SetupNext)
        };
        actions = actions.push(self::action(label, message, palette, ButtonKind::Primary, enabled));

        let mut body = column![heading, steps, scrollable(content).height(Length::Fill).spacing(10)].spacing(24);
        if let Some(error) = &self.error {
            body = body.push(text(error.clone()).size(12).color(palette.purple));
        }
        if self.busy {
            body = body.push(self::note("Working... Keep Gecko open.", palette));
        }
        let body = body.push(column![self::divider(palette.border), actions].spacing(16));

        opaque(overlay::modal(
            palette,
            600.0,
            24.0,
            Message::Noop,
            container(body).height(466).into(),
        ))
        .into()
    }

    fn path_row(&self, field: Field, palette: &Palette) -> Element<'static, Message> {
        let (label, hint) = field.info();
        let path = self.sources.path(field);

        let value: Element<'static, Message> = match path {
            Some(path) => self::path_text(path, palette),
            None => text("No selection").size(12).color(palette.text_mute).into(),
        };
        let info = column![
            row![
                text(label).size(13).font(self::semibold()).color(palette.text),
                text(hint).size(11).color(palette.text_mute)
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            value,
        ]
        .spacing(6)
        .width(Length::Fill);

        let mut controls = row![].spacing(4).align_y(Alignment::Center);
        if path.is_some() {
            controls = controls.push(self::action(
                "Clear",
                Message::SetupClear(field),
                palette,
                ButtonKind::Quiet,
                !self.busy,
            ));
        } else {
            controls = controls.push(Space::new().width(53));
        }
        controls = controls.push(self::action(
            if path.is_some() { "Change..." } else { "Choose..." },
            Message::SetupPick(field),
            palette,
            ButtonKind::Secondary,
            !self.busy,
        ));

        container(row![info, controls].spacing(14).align_y(Alignment::Center))
            .padding([14, 0])
            .into()
    }
}

fn semibold() -> iced::Font {
    iced::Font {
        weight: iced::font::Weight::Semibold,
        ..iced::Font::DEFAULT
    }
}

fn divider(color: Color) -> Element<'static, Message> {
    container(Space::new())
        .height(1)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(color.into()),
            ..Default::default()
        })
        .into()
}

fn note(message: impl text::IntoFragment<'static>, palette: &Palette) -> Element<'static, Message> {
    text(message).size(12).color(palette.text_dim).into()
}

fn path_text(path: &Path, palette: &Palette) -> Element<'static, Message> {
    let full = path.display().to_string();
    let short = match full.char_indices().nth_back(44) {
        Some((start, _)) if full.chars().count() > 46 => format!("...{}", &full[start..]),
        _ => full,
    };

    self::note(short, palette)
}

fn summary_row(label: &'static str, path: &Path, palette: &Palette) -> Element<'static, Message> {
    column![
        text(label).size(13).font(self::semibold()).color(palette.text),
        self::path_text(path, palette)
    ]
    .spacing(5)
    .into()
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Primary,
    Secondary,
    Quiet,
}

fn action(
    label: &'static str,
    message: Message,
    palette: &Palette,
    kind: ButtonKind,
    enabled: bool,
) -> Element<'static, Message> {
    let p = *palette;

    button(text(label).size(12).font(self::semibold()))
        .padding([8, 12])
        .on_press_maybe(enabled.then_some(message))
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let disabled = matches!(status, button::Status::Disabled);
            let (background, foreground, border) = match kind {
                ButtonKind::Primary => (if hovered { p.text_dim } else { p.text }, p.bg, Color::TRANSPARENT),
                ButtonKind::Secondary => (if hovered { p.surface_2 } else { p.surface }, p.text, p.border_2),
                ButtonKind::Quiet => (
                    if hovered { p.surface } else { Color::TRANSPARENT },
                    p.text_dim,
                    Color::TRANSPARENT,
                ),
            };

            button::Style {
                background: Some(Background::Color(if disabled { p.surface } else { background })),
                text_color: if disabled { p.text_mute } else { foreground },
                border: Border {
                    color: border,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

pub async fn pick(field: Field) -> Option<PathBuf> {
    let dialog = rfd::AsyncFileDialog::new().set_title(field.info().0);

    let handle = match field {
        Field::Dsp | Field::Coef | Field::Ipl => dialog.pick_file().await,
        _ => dialog.pick_folder().await,
    };
    handle.map(|h| h.path().to_path_buf())
}

fn read_rom(path: &Path, size: usize) -> anyhow::Result<Vec<u8>> {
    let metadata = fs::metadata(path).with_context(|| format!("Cannot read {}", path.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() == size as u64,
        "{} must be a {}-byte ROM file",
        path.display(),
        size
    );

    let data = fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    ensure!(
        data.len() == size,
        "{} changed while being read; select it again",
        path.display()
    );
    Ok(data)
}

fn system_files(sources: &Sources) -> anyhow::Result<Vec<(&'static str, Vec<u8>)>> {
    let dsp = sources
        .dsp
        .as_deref()
        .context("Choose the DSP ROM before continuing.")?;
    let coef = sources
        .coef
        .as_deref()
        .context("Choose the DSP coefficient ROM before continuing.")?;

    let mut files = vec![
        (DSP_ROM_FILE, self::read_rom(dsp, 8192)?),
        (DSP_COEF_FILE, self::read_rom(coef, 4096)?),
    ];

    if let Some(path) = &sources.ipl {
        let mut data = self::read_rom(path, image::ipl::IPL_ROM_SIZE)?;
        if image::ipl::is_encoded(&data) {
            image::ipl::scramble(&mut data);
        }
        ensure!(
            !image::ipl::is_encoded(&data),
            "{} is not a recognized encoded or decoded GameCube IPL",
            path.display()
        );
        files.push((IPL_FILE, data));
    }

    Ok(files)
}

pub fn validate_step(sources: &Sources, step: Step) -> anyhow::Result<()> {
    match step {
        Step::System => self::system_files(sources).map(drop),
        Step::Folders => self::validate_folders(sources),
        Step::Finish => Ok(()),
    }
}

fn validate_folders(sources: &Sources) -> anyhow::Result<()> {
    for path in [&sources.gcn, &sources.wii].into_iter().flatten() {
        ensure!(path.is_dir(), "Game folder does not exist: {}", path.display());
    }

    ensure!(
        sources.gcn.is_none() || sources.ipl.is_some(),
        "Choose a GameCube IPL in step 1 to use a GameCube library."
    );
    Ok(())
}

fn stage_nand(source: &Path, destination: &Path) -> anyhow::Result<Option<tempfile::TempDir>> {
    let source = fs::canonicalize(source).context("Cannot open the selected NAND folder")?;
    if destination.exists() && fs::canonicalize(destination)? == source {
        return Ok(None);
    }

    ensure!(
        !destination.exists(),
        "Wii storage already exists at {}. Leave NAND unselected to keep it. Importing requires an unused destination.",
        destination.display()
    );
    ensure!(
        source.join("title").is_dir() && source.join("shared2").is_dir(),
        "Choose a NAND root containing title/ and shared2/."
    );

    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    ensure!(
        !fs::canonicalize(parent)?.starts_with(&source),
        "The NAND destination cannot be inside the source folder."
    );

    let stage = tempfile::tempdir_in(parent)?;
    let root = stage.path().join("nand");
    fs::create_dir(&root)?;

    for entry in walkdir::WalkDir::new(&source).follow_links(false) {
        let entry = entry.context("Cannot read a NAND entry")?;
        let target = root.join(entry.path().strip_prefix(&source)?);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), &target).with_context(|| format!("Cannot copy {}", entry.path().display()))?;
        } else {
            bail!(
                "NAND contains a symbolic link or special file: {}",
                entry.path().display()
            );
        }
    }

    Ok(Some(stage))
}

pub fn install(sources: Sources, mut cfg: Config, config_path: &Path, nand_dest: &Path) -> anyhow::Result<Config> {
    let files = self::system_files(&sources)?;
    self::validate_folders(&sources)?;

    let nand_stage = sources
        .nand
        .as_deref()
        .map(|p| self::stage_nand(p, nand_dest))
        .transpose()?
        .flatten();

    let system = cfg.system_dir_resolved();
    fs::create_dir_all(&system).with_context(|| format!("Cannot create {}", system.display()))?;

    let mut staged = Vec::new();
    for (name, data) in files {
        let mut file = tempfile::NamedTempFile::new_in(&system)?;
        file.write_all(&data)?;
        file.as_file().sync_all()?;
        staged.push((system.join(name), file));
    }

    for (path, file) in staged {
        file.persist(&path)
            .with_context(|| format!("Cannot install {}", path.display()))?;
    }

    if let Some(stage) = nand_stage {
        ensure!(
            !nand_dest.exists(),
            "Wii storage was created during setup; it has not been replaced."
        );
        fs::rename(stage.path().join("nand"), nand_dest).context("Cannot install Wii NAND")?;
    }

    cfg.dsp_rom = Some(system.join(DSP_ROM_FILE));
    cfg.dsp_coef = Some(system.join(DSP_COEF_FILE));
    if sources.ipl.is_some() {
        cfg.ipl = Some(system.join(IPL_FILE));
    }
    cfg.gcn_library = sources.gcn;
    cfg.wii_library = sources.wii;
    cfg.setup_completed = true;

    config::save(config_path, &cfg).context("Files were installed, but settings could not be saved. Check permissions on config.toml. If retrying, leave NAND unselected.")?;
    Ok(cfg)
}

pub async fn run_install(sources: Sources, cfg: Config) -> Result<Config, String> {
    let path = config::config_path();
    let nand = gecko::paths::fs_root();

    tokio::task::spawn_blocking(move || self::install(sources, cfg, &path, &nand).map_err(|e| format!("{e:#}")))
        .await
        .map_err(|e| e.to_string())?
}
