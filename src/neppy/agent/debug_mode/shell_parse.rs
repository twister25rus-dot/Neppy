//! A deliberately small, conservative splitter for shell command strings.
//!
//! It does not execute or fully interpret anything: it only has to find every
//! simple command a string could run so `policy` can classify each one. Anything
//! that could hide a command from a word-level scan sets `complex`, and the
//! policy then asks instead of allowing.

/// Result of [`parse`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Simple commands, each a list of unquoted words.
    pub segments: Vec<Vec<String>>,
    /// A command substitution sits inside double quotes, where the word-level
    /// scan cannot see the command it runs.
    pub complex: bool,
}

/// Splits `cmd` on `&&`, `||`, `;`, `|`, `&`, newlines, subshell parens and
/// backticks (outside quotes), so `a && (b; c) | d` yields four segments.
/// `Err` for an unterminated quote or a trailing escape.
pub fn parse(cmd: &str) -> Result<Parsed, ()> {
    let mut out = Parsed::default();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;

    fn end_word(words: &mut Vec<String>, cur: &mut String, in_word: &mut bool) {
        if *in_word {
            words.push(std::mem::take(cur));
            *in_word = false;
        }
    }
    fn end_segment(out: &mut Parsed, words: &mut Vec<String>) {
        if !words.is_empty() {
            out.segments.push(std::mem::take(words));
        }
    }

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i += 1;
                let next = *chars.get(i).ok_or(())?;
                if next != '\n' {
                    cur.push(next);
                    in_word = true;
                }
            }
            '\'' => {
                in_word = true;
                i += 1;
                loop {
                    let ch = *chars.get(i).ok_or(())?;
                    if ch == '\'' {
                        break;
                    }
                    cur.push(ch);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                loop {
                    let ch = *chars.get(i).ok_or(())?;
                    match ch {
                        '"' => break,
                        '\\' => {
                            i += 1;
                            cur.push(*chars.get(i).ok_or(())?);
                        }
                        '`' => {
                            out.complex = true;
                            cur.push(ch);
                        }
                        '$' if chars.get(i + 1) == Some(&'(') => {
                            out.complex = true;
                            cur.push(ch);
                        }
                        _ => cur.push(ch),
                    }
                    i += 1;
                }
            }
            '#' if !in_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            c if c.is_whitespace() && c != '\n' => end_word(&mut words, &mut cur, &mut in_word),
            ';' | '&' | '|' | '\n' | '(' | ')' | '`' => {
                end_word(&mut words, &mut cur, &mut in_word);
                end_segment(&mut out, &mut words);
            }
            _ => {
                cur.push(c);
                in_word = true;
            }
        }
        i += 1;
    }
    end_word(&mut words, &mut cur, &mut in_word);
    end_segment(&mut out, &mut words);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(s: &str) -> Vec<Vec<String>> {
        parse(s).unwrap().segments
    }

    #[test]
    fn splits_on_operators_and_subshells() {
        assert_eq!(segs("a && b; c | d || e & f").len(), 6);
        assert_eq!(segs("echo $(rm -rf x)")[1], vec!["rm", "-rf", "x"]);
        assert_eq!(segs("(cd x && make)").len(), 2);
        assert_eq!(segs("a\nb").len(), 2);
    }

    #[test]
    fn quotes_group_words_and_hide_operators() {
        assert_eq!(
            segs("git commit -m 'a && b; c'"),
            vec![vec!["git", "commit", "-m", "a && b; c"]]
        );
        assert_eq!(segs("echo \"x;y\" z"), vec![vec!["echo", "x;y", "z"]]);
        assert_eq!(segs("rm\\ -rf"), vec![vec!["rm -rf"]]);
    }

    #[test]
    fn comments_are_dropped_and_substitution_in_quotes_is_flagged() {
        assert_eq!(segs("ls # sudo rm"), vec![vec!["ls"]]);
        assert_eq!(segs("a#b"), vec![vec!["a#b"]]);
        assert!(parse("echo \"$(date)\"").unwrap().complex);
        assert!(parse("echo \"`date`\"").unwrap().complex);
        assert!(!parse("echo '$(date)'").unwrap().complex);
    }

    #[test]
    fn unterminated_input_is_an_error() {
        assert!(parse("echo 'a").is_err());
        assert!(parse("echo \"a").is_err());
        assert!(parse("echo a\\").is_err());
    }
}
