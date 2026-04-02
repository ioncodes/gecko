use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Context, RichText, ScrollArea, TextBuffer, TextEdit, TextStyle, Ui};

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil",
    "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const LUA_BUILTINS: &[&str] = &[
    "log",
    "print",
    "string",
    "table",
    "math",
    "pairs",
    "ipairs",
    "tonumber",
    "tostring",
    "type",
    "error",
    "assert",
    "pcall",
    "xpcall",
    "select",
    "unpack",
    "require",
    "format",
    "refresh_traps",
    "traps",
];

fn highlight_lua(ui: &Ui, text: &dyn TextBuffer, wrap_width: f32) -> Arc<egui::Galley> {
    let src = text.as_str();
    let font_id = TextStyle::Monospace.resolve(ui.style());
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let default_fmt = TextFormat::simple(font_id.clone(), Color32::from_rgb(220, 220, 220));
    let keyword_fmt = TextFormat::simple(font_id.clone(), Color32::from_rgb(200, 120, 220));
    let builtin_fmt = TextFormat::simple(font_id.clone(), Color32::from_rgb(100, 180, 255));
    let string_fmt = TextFormat::simple(font_id.clone(), Color32::from_rgb(150, 220, 150));
    let number_fmt = TextFormat::simple(font_id.clone(), Color32::from_rgb(255, 200, 100));
    let comment_fmt = TextFormat::simple(font_id.clone(), Color32::from_rgb(120, 120, 120));

    let bytes = src.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Line comments: -- (and block comments --[[ ... ]])
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            let start = i;
            if i + 3 < len && bytes[i + 2] == b'[' && bytes[i + 3] == b'[' {
                i += 4;
                while i + 1 < len {
                    if bytes[i] == b']' && bytes[i + 1] == b']' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            job.append(&src[start..i], 0.0, comment_fmt.clone());
            continue;
        }

        // Strings: "..." or '...'
        if b == b'"' || b == b'\'' {
            let quote = b;
            let start = i;
            i += 1;
            while i < len && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            job.append(&src[start..i], 0.0, string_fmt.clone());
            continue;
        }

        // Block strings: [[...]]
        if b == b'[' && i + 1 < len && bytes[i + 1] == b'[' {
            let start = i;
            i += 2;
            while i + 1 < len {
                if bytes[i] == b']' && bytes[i + 1] == b']' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            job.append(&src[start..i], 0.0, string_fmt.clone());
            continue;
        }

        // Numbers
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            if b == b'0' && i + 1 < len && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                i += 2;
                while i < len && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
            } else {
                while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < len && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            job.append(&src[start..i], 0.0, number_fmt.clone());
            continue;
        }

        // Identifiers / keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            let fmt = if LUA_KEYWORDS.contains(&word) {
                &keyword_fmt
            } else if LUA_BUILTINS.contains(&word) {
                &builtin_fmt
            } else {
                &default_fmt
            };
            job.append(word, 0.0, fmt.clone());
            continue;
        }

        // Everything else (handles multi-byte UTF-8)
        let ch = &src[i..];
        let c = ch.chars().next().unwrap();
        job.append(&src[i..i + c.len_utf8()], 0.0, default_fmt.clone());
        i += c.len_utf8();
    }

    ui.fonts_mut(|f| f.layout_job(job))
}

pub fn show_lua(
    ctx: &Context,
    open: &mut bool,
    source: &mut String,
    logs: &[String],
    load_script: &mut bool,
    clear_log: &mut bool,
) {
    egui::Window::new("Lua").open(open).show(ctx, |ui| {
        use egui_phosphor::regular as icons;

        ui.horizontal(|ui| {
            if ui.button(format!("{} Load", icons::PLAY)).clicked() {
                *load_script = true;
            }
            if ui.button(format!("{} Clear Log", icons::TRASH)).clicked() {
                *clear_log = true;
            }
        });

        ui.separator();

        let avail = ui.available_height();
        let editor_height = (avail * 0.6).max(120.0);

        let mut layouter = |ui: &Ui, buf: &dyn TextBuffer, wrap_width: f32| highlight_lua(ui, buf, wrap_width);

        ScrollArea::vertical()
            .id_salt("lua_editor_scroll")
            .max_height(editor_height)
            .auto_shrink(false)
            .show(ui, |ui| {
                TextEdit::multiline(source)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .layouter(&mut layouter)
                    .show(ui);
            });

        ui.separator();

        ScrollArea::vertical()
            .id_salt("lua_log_scroll")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in logs {
                    ui.label(RichText::new(line).monospace().color(Color32::from_rgb(200, 200, 200)));
                }
            });
    });
}
