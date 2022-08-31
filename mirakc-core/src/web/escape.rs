// Took from https://github.com/rust-lang/rust/blob/master/src/librustdoc/html/escape.rs
#[inline(always)]
pub(super) fn escape<'a>(str: &'a str) -> Escape<'a> {
    Escape(str)
}

pub(super) struct Escape<'a>(pub &'a str);

impl<'a> std::fmt::Display for Escape<'a> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Because the internet is always right, turns out there's not that many
        // characters to escape: http://stackoverflow.com/questions/7381974
        let Escape(s) = *self;
        let pile_o_bits = s;
        let mut last = 0;
        for (i, ch) in s.bytes().enumerate() {
            match ch as char {
                '<' | '>' | '&' | '\'' | '"' => {
                    fmt.write_str(&pile_o_bits[last..i])?;
                    let s = match ch as char {
                        '>' => "&gt;",
                        '<' => "&lt;",
                        '&' => "&amp;",
                        '\'' => "&#39;",
                        '"' => "&quot;",
                        _ => unreachable!(),
                    };
                    fmt.write_str(s)?;
                    last = i + 1;
                }
                _ => {}
            }
        }

        if last < s.len() {
            fmt.write_str(&pile_o_bits[last..])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        assert_eq!("a&lt;a&gt;a&amp;a&#39;a&quot;a",
                   format!("{}", escape(r#"a<a>a&a'a"a"#)));
    }
}
