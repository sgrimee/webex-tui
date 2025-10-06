//! Theme data models

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use super::parser::SerializableColor;

/// Complete theme definition including palette, semantic roles, and user colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    /// Theme name for identification
    pub name: String,
    /// Base color palette (16 ANSI colors)
    #[serde(default)]
    pub palette: Palette,
    /// Semantic color role assignments
    #[serde(default)]
    pub roles: Roles,
    /// Colors used for cycling message sender colors
    #[serde(default = "default_user_colors")]
    pub user_colors: Vec<SerializableColor>,
}

/// Standard 16-color terminal palette
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Palette {
    #[serde(default = "default_black")]
    pub black: SerializableColor,
    #[serde(default = "default_red")]
    pub red: SerializableColor,
    #[serde(default = "default_green")]
    pub green: SerializableColor,
    #[serde(default = "default_yellow")]
    pub yellow: SerializableColor,
    #[serde(default = "default_blue")]
    pub blue: SerializableColor,
    #[serde(default = "default_magenta")]
    pub magenta: SerializableColor,
    #[serde(default = "default_cyan")]
    pub cyan: SerializableColor,
    #[serde(default = "default_white")]
    pub white: SerializableColor,
    #[serde(default = "default_bright_black")]
    pub bright_black: SerializableColor,
    #[serde(default = "default_bright_red")]
    pub bright_red: SerializableColor,
    #[serde(default = "default_bright_green")]
    pub bright_green: SerializableColor,
    #[serde(default = "default_bright_yellow")]
    pub bright_yellow: SerializableColor,
    #[serde(default = "default_bright_blue")]
    pub bright_blue: SerializableColor,
    #[serde(default = "default_bright_magenta")]
    pub bright_magenta: SerializableColor,
    #[serde(default = "default_bright_cyan")]
    pub bright_cyan: SerializableColor,
    #[serde(default = "default_bright_white")]
    pub bright_white: SerializableColor,
}

/// Semantic color role assignments for UI elements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Roles {
    #[serde(default = "default_background")]
    pub background: SerializableColor,
    #[serde(default = "default_text_primary")]
    pub text_primary: SerializableColor,
    #[serde(default = "default_text_muted")]
    pub text_muted: SerializableColor,
    #[serde(default = "default_accent")]
    pub accent: SerializableColor,
    #[serde(default = "default_selection_bg")]
    pub selection_bg: SerializableColor,
    #[serde(default = "default_selection_fg")]
    pub selection_fg: SerializableColor,
    #[serde(default = "default_border")]
    pub border: SerializableColor,
    #[serde(default = "default_border_active")]
    pub border_active: SerializableColor,
    #[serde(default = "default_title")]
    pub title: SerializableColor,
    #[serde(default = "default_hint")]
    pub hint: SerializableColor,
    #[serde(default = "default_room_unread")]
    pub room_unread: SerializableColor,
    #[serde(default = "default_room_team")]
    pub room_team: SerializableColor,
    #[serde(default = "default_msg_timestamp")]
    pub msg_timestamp: SerializableColor,
    #[serde(default = "default_log_error")]
    pub log_error: SerializableColor,
    #[serde(default = "default_log_warn")]
    pub log_warn: SerializableColor,
    #[serde(default = "default_log_info")]
    pub log_info: SerializableColor,
    #[serde(default = "default_log_debug")]
    pub log_debug: SerializableColor,
    #[serde(default = "default_log_trace")]
    pub log_trace: SerializableColor,
    #[serde(default = "default_compose_status")]
    pub compose_status: SerializableColor,
}

impl Roles {
    // Convenience methods to get Color values for UI usage
    #[allow(dead_code)]
    pub fn background(&self) -> Color { self.background.0 }
    #[allow(dead_code)]
    pub fn text_primary(&self) -> Color { self.text_primary.0 }
    pub fn text_muted(&self) -> Color { self.text_muted.0 }
    pub fn accent(&self) -> Color { self.accent.0 }
    pub fn selection_bg(&self) -> Color { self.selection_bg.0 }
    pub fn selection_fg(&self) -> Color { self.selection_fg.0 }
    pub fn border(&self) -> Color { self.border.0 }
    pub fn border_active(&self) -> Color { self.border_active.0 }
    pub fn title(&self) -> Color { self.title.0 }
    pub fn hint(&self) -> Color { self.hint.0 }
    pub fn room_unread(&self) -> Color { self.room_unread.0 }
    pub fn room_team(&self) -> Color { self.room_team.0 }
    pub fn msg_timestamp(&self) -> Color { self.msg_timestamp.0 }
    pub fn log_error(&self) -> Color { self.log_error.0 }
    pub fn log_warn(&self) -> Color { self.log_warn.0 }
    pub fn log_info(&self) -> Color { self.log_info.0 }
    pub fn log_debug(&self) -> Color { self.log_debug.0 }
    pub fn log_trace(&self) -> Color { self.log_trace.0 }
    pub fn compose_status(&self) -> Color { self.compose_status.0 }
}

impl Theme {
    /// Get colors for user cycling (converting to Color)
    pub fn user_colors(&self) -> Vec<Color> {
        self.user_colors.iter().map(|c| c.0).collect()
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            palette: Palette::default(),
            roles: Roles::default(),
            user_colors: default_user_colors(),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            black: default_black(),
            red: default_red(),
            green: default_green(),
            yellow: default_yellow(),
            blue: default_blue(),
            magenta: default_magenta(),
            cyan: default_cyan(),
            white: default_white(),
            bright_black: default_bright_black(),
            bright_red: default_bright_red(),
            bright_green: default_bright_green(),
            bright_yellow: default_bright_yellow(),
            bright_blue: default_bright_blue(),
            bright_magenta: default_bright_magenta(),
            bright_cyan: default_bright_cyan(),
            bright_white: default_bright_white(),
        }
    }
}

impl Default for Roles {
    fn default() -> Self {
        Self {
            background: default_background(),
            text_primary: default_text_primary(),
            text_muted: default_text_muted(),
            accent: default_accent(),
            selection_bg: default_selection_bg(),
            selection_fg: default_selection_fg(),
            border: default_border(),
            border_active: default_border_active(),
            title: default_title(),
            hint: default_hint(),
            room_unread: default_room_unread(),
            room_team: default_room_team(),
            msg_timestamp: default_msg_timestamp(),
            log_error: default_log_error(),
            log_warn: default_log_warn(),
            log_info: default_log_info(),
            log_debug: default_log_debug(),
            log_trace: default_log_trace(),
            compose_status: default_compose_status(),
        }
    }
}

// Default color functions for palette
fn default_black() -> SerializableColor { SerializableColor(Color::Black) }
fn default_red() -> SerializableColor { SerializableColor(Color::Red) }
fn default_green() -> SerializableColor { SerializableColor(Color::Green) }
fn default_yellow() -> SerializableColor { SerializableColor(Color::Yellow) }
fn default_blue() -> SerializableColor { SerializableColor(Color::Blue) }
fn default_magenta() -> SerializableColor { SerializableColor(Color::Magenta) }
fn default_cyan() -> SerializableColor { SerializableColor(Color::Cyan) }
fn default_white() -> SerializableColor { SerializableColor(Color::White) }
fn default_bright_black() -> SerializableColor { SerializableColor(Color::DarkGray) }
fn default_bright_red() -> SerializableColor { SerializableColor(Color::LightRed) }
fn default_bright_green() -> SerializableColor { SerializableColor(Color::LightGreen) }
fn default_bright_yellow() -> SerializableColor { SerializableColor(Color::LightYellow) }
fn default_bright_blue() -> SerializableColor { SerializableColor(Color::LightBlue) }
fn default_bright_magenta() -> SerializableColor { SerializableColor(Color::LightMagenta) }
fn default_bright_cyan() -> SerializableColor { SerializableColor(Color::LightCyan) }
fn default_bright_white() -> SerializableColor { SerializableColor(Color::Gray) }

// Default color functions for roles - mapping current hardcoded colors
fn default_background() -> SerializableColor { SerializableColor(Color::Reset) }
fn default_text_primary() -> SerializableColor { SerializableColor(Color::Reset) }
fn default_text_muted() -> SerializableColor { SerializableColor(Color::Gray) }
fn default_accent() -> SerializableColor { SerializableColor(Color::LightCyan) }
fn default_selection_bg() -> SerializableColor { SerializableColor(Color::Yellow) }
fn default_selection_fg() -> SerializableColor { SerializableColor(Color::Black) }
fn default_border() -> SerializableColor { SerializableColor(Color::Reset) }
fn default_border_active() -> SerializableColor { SerializableColor(Color::Cyan) }
fn default_title() -> SerializableColor { SerializableColor(Color::LightCyan) }
fn default_hint() -> SerializableColor { SerializableColor(Color::Gray) }
fn default_room_unread() -> SerializableColor { SerializableColor(Color::LightBlue) }
fn default_room_team() -> SerializableColor { SerializableColor(Color::LightCyan) }
fn default_msg_timestamp() -> SerializableColor { SerializableColor(Color::Gray) }
fn default_log_error() -> SerializableColor { SerializableColor(Color::Red) }
fn default_log_warn() -> SerializableColor { SerializableColor(Color::Yellow) }
fn default_log_info() -> SerializableColor { SerializableColor(Color::Blue) }
fn default_log_debug() -> SerializableColor { SerializableColor(Color::Green) }
fn default_log_trace() -> SerializableColor { SerializableColor(Color::Gray) }
fn default_compose_status() -> SerializableColor { SerializableColor(Color::Yellow) }

fn default_user_colors() -> Vec<SerializableColor> {
    vec![
        SerializableColor(Color::Red),
        SerializableColor(Color::Green),
        SerializableColor(Color::Yellow),
        SerializableColor(Color::Blue),
        SerializableColor(Color::Magenta),
        SerializableColor(Color::Cyan),
        SerializableColor(Color::Gray),
        SerializableColor(Color::LightRed),
        SerializableColor(Color::LightGreen),
        SerializableColor(Color::LightYellow),
        SerializableColor(Color::LightBlue),
        SerializableColor(Color::LightMagenta),
        SerializableColor(Color::LightCyan),
    ]
}