//! Theme file parsing utilities

use ratatui::style::Color;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Error type for color parsing failures
#[derive(Debug, thiserror::Error)]
pub enum ColorParseError {
    #[error("Invalid hex color format: {0}")]
    InvalidHex(String),
    #[error("Unknown color name: {0}")]
    UnknownName(String),
    #[error("Invalid RGB values: {0}")]
    #[allow(dead_code)]
    InvalidRgb(String),
}

/// Parse a color from various string formats
pub fn parse_color(input: &str) -> Result<Color, ColorParseError> {
    let input = input.trim();

    // Handle hex colors
    if input.starts_with('#') {
        return parse_hex_color(input);
    }

    // Handle named colors
    parse_named_color(input)
}

/// Parse hex color in format #RRGGBB or #RGB
fn parse_hex_color(hex: &str) -> Result<Color, ColorParseError> {
    if hex.len() == 7 {
        // #RRGGBB format
        let r = u8::from_str_radix(&hex[1..3], 16)
            .map_err(|_| ColorParseError::InvalidHex(hex.to_string()))?;
        let g = u8::from_str_radix(&hex[3..5], 16)
            .map_err(|_| ColorParseError::InvalidHex(hex.to_string()))?;
        let b = u8::from_str_radix(&hex[5..7], 16)
            .map_err(|_| ColorParseError::InvalidHex(hex.to_string()))?;
        Ok(Color::Rgb(r, g, b))
    } else if hex.len() == 4 {
        // #RGB format - expand to #RRGGBB
        let r = u8::from_str_radix(&hex[1..2], 16)
            .map_err(|_| ColorParseError::InvalidHex(hex.to_string()))?;
        let g = u8::from_str_radix(&hex[2..3], 16)
            .map_err(|_| ColorParseError::InvalidHex(hex.to_string()))?;
        let b = u8::from_str_radix(&hex[3..4], 16)
            .map_err(|_| ColorParseError::InvalidHex(hex.to_string()))?;
        Ok(Color::Rgb(r * 17, g * 17, b * 17)) // 17 = 255/15
    } else {
        Err(ColorParseError::InvalidHex(hex.to_string()))
    }
}

/// Parse named color (case-insensitive)
fn parse_named_color(name: &str) -> Result<Color, ColorParseError> {
    match name.to_lowercase().as_str() {
        "black" => Ok(Color::Black),
        "red" => Ok(Color::Red),
        "green" => Ok(Color::Green),
        "yellow" => Ok(Color::Yellow),
        "blue" => Ok(Color::Blue),
        "magenta" => Ok(Color::Magenta),
        "cyan" => Ok(Color::Cyan),
        "gray" | "grey" => Ok(Color::Gray),
        "white" => Ok(Color::White),
        "dark_gray" | "dark_grey" | "bright_black" => Ok(Color::DarkGray),
        "light_red" | "bright_red" => Ok(Color::LightRed),
        "light_green" | "bright_green" => Ok(Color::LightGreen),
        "light_yellow" | "bright_yellow" => Ok(Color::LightYellow),
        "light_blue" | "bright_blue" => Ok(Color::LightBlue),
        "light_magenta" | "bright_magenta" => Ok(Color::LightMagenta),
        "light_cyan" | "bright_cyan" => Ok(Color::LightCyan),
        "reset" => Ok(Color::Reset),
        _ => Err(ColorParseError::UnknownName(name.to_string())),
    }
}

/// Custom deserializer for Color that handles multiple formats
pub fn deserialize_color<'de, D>(deserializer: D) -> Result<Color, D::Error>
where
    D: Deserializer<'de>,
{
    // Try to deserialize as string first
    let value = String::deserialize(deserializer)?;
    parse_color(&value).map_err(|e| D::Error::custom(format!("Failed to parse color: {e}")))
}

/// Wrapper for Color that implements serde traits
#[derive(Debug, Clone, PartialEq)]
pub struct SerializableColor(pub Color);

impl From<Color> for SerializableColor {
    fn from(color: Color) -> Self {
        SerializableColor(color)
    }
}

impl From<SerializableColor> for Color {
    fn from(sc: SerializableColor) -> Self {
        sc.0
    }
}

impl<'de> Deserialize<'de> for SerializableColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let color = deserialize_color(deserializer)?;
        Ok(SerializableColor(color))
    }
}

impl Serialize for SerializableColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize Color as hex string when possible
        match self.0 {
            Color::Rgb(r, g, b) => {
                let hex = format!("#{r:02x}{g:02x}{b:02x}");
                serializer.serialize_str(&hex)
            }
            Color::Reset => serializer.serialize_str("reset"),
            Color::Black => serializer.serialize_str("black"),
            Color::Red => serializer.serialize_str("red"),
            Color::Green => serializer.serialize_str("green"),
            Color::Yellow => serializer.serialize_str("yellow"),
            Color::Blue => serializer.serialize_str("blue"),
            Color::Magenta => serializer.serialize_str("magenta"),
            Color::Cyan => serializer.serialize_str("cyan"),
            Color::Gray => serializer.serialize_str("gray"),
            Color::DarkGray => serializer.serialize_str("dark_gray"),
            Color::LightRed => serializer.serialize_str("light_red"),
            Color::LightGreen => serializer.serialize_str("light_green"),
            Color::LightYellow => serializer.serialize_str("light_yellow"),
            Color::LightBlue => serializer.serialize_str("light_blue"),
            Color::LightMagenta => serializer.serialize_str("light_magenta"),
            Color::LightCyan => serializer.serialize_str("light_cyan"),
            Color::White => serializer.serialize_str("white"),
            Color::Indexed(i) => serializer.serialize_str(&format!("indexed_{i}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_colors() {
        assert_eq!(parse_color("#ff0000").unwrap(), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("#00ff00").unwrap(), Color::Rgb(0, 255, 0));
        assert_eq!(parse_color("#0000ff").unwrap(), Color::Rgb(0, 0, 255));
        assert_eq!(parse_color("#f00").unwrap(), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("#0f0").unwrap(), Color::Rgb(0, 255, 0));
        assert_eq!(parse_color("#00f").unwrap(), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn test_parse_named_colors() {
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("RED").unwrap(), Color::Red);
        assert_eq!(parse_color("light_blue").unwrap(), Color::LightBlue);
        assert_eq!(parse_color("bright_red").unwrap(), Color::LightRed);
        assert_eq!(parse_color("gray").unwrap(), Color::Gray);
        assert_eq!(parse_color("grey").unwrap(), Color::Gray);
    }

    #[test]
    fn test_invalid_colors() {
        assert!(parse_color("#gg0000").is_err());
        assert!(parse_color("#ff00").is_err());
        assert!(parse_color("invalid_color").is_err());
        assert!(parse_color("").is_err());
    }
}
