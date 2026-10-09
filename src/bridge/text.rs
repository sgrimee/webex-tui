//! Terminal-safe presentation text; no HTML, links or attachment fetches.

/// Strip ANSI/OSC and C0/C1 controls, preserving newline/tab for message bodies.
pub(super) fn body_text(value: &str) -> String {
    enum State {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }
    let mut state = State::Text;
    let mut output = String::new();
    for ch in value.chars() {
        state = match state {
            State::Text => match ch {
                '\u{1b}' => State::Escape,
                '\u{9b}' => State::Csi,
                '\u{9d}' => State::Osc,
                _ => {
                    if !ch.is_control() || matches!(ch, '\n' | '\t') {
                        output.push(ch);
                    }
                    State::Text
                }
            },
            State::Escape => match ch {
                '[' => State::Csi,
                ']' => State::Osc,
                '\u{1b}' => State::Escape,
                ' '..='/' => State::Escape,
                _ => State::Text,
            },
            State::Csi => match ch {
                '\u{1b}' => State::Escape,
                '@'..='~' => State::Text,
                _ => State::Csi,
            },
            State::Osc => match ch {
                '\u{7}' | '\u{9c}' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
            State::OscEscape => match ch {
                '\\' | '\u{7}' | '\u{9c}' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
        };
    }
    output
}

pub(super) fn label_text(value: String) -> String {
    body_text(&value).replace(['\n', '\t'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_osc_charset_and_c1_sequences_without_fetching_anything() {
        assert_eq!(body_text("a\x1b[31mb\x1b[0m\x1b]8;;https://example.invalid\x1b\\c\x1b]8;;\x07\x1b(Bd\u{9b}2Je\u{9d}52;clipboard\u{9c}f"), "abcdef");
    }

    #[test]
    fn preserves_body_line_breaks_but_not_label_line_breaks_or_controls() {
        assert_eq!(body_text("a\n\tb\r\0\x07"), "a\n\tb");
        assert_eq!(label_text("a\n\tb\r".into()), "a  b");
        assert_eq!(body_text("safe\x1b]unfinished OSC"), "safe");
    }
}
