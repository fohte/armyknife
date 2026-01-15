/// Adds an indent prefix to the beginning of each line in the text.
///
/// # Arguments
/// * `text` - The text to indent
/// * `indent` - The string to prepend to each line
///
/// # Returns
/// The indented text with each line prefixed by the indent string.
#[allow(dead_code)]
pub fn indent_text(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_line() {
        assert_eq!(indent_text("hello", "  "), "  hello");
    }

    #[test]
    fn test_multiple_lines() {
        let input = "line1\nline2\nline3";
        let expected = "  line1\n  line2\n  line3";
        assert_eq!(indent_text(input, "  "), expected);
    }

    #[test]
    fn test_custom_indent() {
        let input = "a\nb";
        assert_eq!(indent_text(input, ">>> "), ">>> a\n>>> b");
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(indent_text("", "  "), "");
    }

    #[test]
    fn test_empty_indent() {
        assert_eq!(indent_text("hello\nworld", ""), "hello\nworld");
    }

    #[test]
    fn test_with_empty_lines() {
        let input = "first\n\nthird";
        let expected = "  first\n  \n  third";
        assert_eq!(indent_text(input, "  "), expected);
    }
}
