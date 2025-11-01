//! Tests for the theme system

#[cfg(test)]
mod tests {
    use super::super::load_theme;
    use crate::theme::models::Theme;
    use crate::theme::parser::parse_color;
    use ratatui::style::Color;

    #[test]
    fn test_default_theme_loads() {
        let theme = load_theme("nonexistent");
        assert_eq!(theme.name, "Default");
        assert!(!theme.user_colors().is_empty());
    }

    #[test]
    fn test_theme_color_parsing() {
        assert_eq!(parse_color("#ff0000").unwrap(), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("#f00").unwrap(), Color::Rgb(255, 0, 0));
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("light_blue").unwrap(), Color::LightBlue);
    }

    #[test]
    fn test_dracula_theme_parsing() {
        let yaml_content = "name: \"Dracula\"
roles:
  background: \"#282a36\"
  accent: \"#bd93f9\"
  room_unread: \"#8be9fd\"
user_colors:
  - \"#ff5555\"
  - \"#50fa7b\"
";

        let theme: Theme = serde_yaml::from_str(yaml_content).unwrap();
        assert_eq!(theme.name, "Dracula");
        assert_eq!(theme.roles.accent(), Color::Rgb(189, 147, 249));
        assert_eq!(theme.user_colors().len(), 2);
        assert_eq!(theme.user_colors()[0], Color::Rgb(255, 85, 85));
    }

    #[test]
    fn test_partial_theme_uses_defaults() {
        let yaml_content = "name: \"Partial\"
roles:
  accent: \"#bd93f9\"
";

        let theme: Theme = serde_yaml::from_str(yaml_content).unwrap();
        assert_eq!(theme.name, "Partial");
        assert_eq!(theme.roles.accent(), Color::Rgb(189, 147, 249));
        // Should use defaults for missing fields
        assert_eq!(theme.roles.hint(), Color::Gray);
        assert!(!theme.user_colors().is_empty());
    }
}
