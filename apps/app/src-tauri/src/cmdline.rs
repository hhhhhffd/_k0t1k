use crate::error::{AppError, CommandResult};

/// Splits a command line into arguments. Single and double quotes group words; there are no
/// escape sequences, so Windows paths keep their backslashes.
pub fn split(line: &str) -> CommandResult<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match (quote, c) {
            (Some(open), c) if c == open => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err(AppError::new(format!("unterminated quote in command line: {line}")));
    }
    if in_word {
        args.push(current);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn splits_words_and_quotes() {
        assert_eq!(split("  a  b ").unwrap(), ["a", "b"]);
        assert_eq!(split(r#"cmd "C:\Program Files\x" --flag='a b'"#).unwrap(), ["cmd", r"C:\Program Files\x", "--flag=a b"]);
        assert_eq!(split(r#"a "" b"#).unwrap(), ["a", "", "b"]);
        assert!(split("").unwrap().is_empty());
    }

    #[test]
    fn rejects_unterminated_quote() {
        assert!(split(r#"a "b"#).is_err());
    }
}
