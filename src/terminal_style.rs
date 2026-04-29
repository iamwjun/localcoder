use std::fmt::{self, Display, Formatter};

use oxink::styles::{BLUE, BOLD, CYAN, DIM, GREEN, RED, StyleCode, WHITE, YELLOW};

#[allow(dead_code)]
pub trait StyleExt: Display {
    fn with_style(&self, style: StyleCode) -> Styled {
        Styled {
            value: self.to_string(),
            style,
        }
    }

    fn bold(&self) -> Styled {
        self.with_style(BOLD)
    }

    fn blue(&self) -> Styled {
        self.with_style(BLUE)
    }

    fn cyan(&self) -> Styled {
        self.with_style(CYAN)
    }

    fn dimmed(&self) -> Styled {
        self.with_style(DIM)
    }

    fn green(&self) -> Styled {
        self.with_style(GREEN)
    }

    fn red(&self) -> Styled {
        self.with_style(RED)
    }

    fn white(&self) -> Styled {
        self.with_style(WHITE)
    }

    fn yellow(&self) -> Styled {
        self.with_style(YELLOW)
    }
}

impl<T: Display + ?Sized> StyleExt for T {}

pub struct Styled {
    value: String,
    style: StyleCode,
}

impl Display for Styled {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "\x1B[{}m{}\x1B[{}m",
            self.style.open, self.value, self.style.close
        )
    }
}

#[cfg(test)]
mod tests {
    use super::StyleExt;

    #[test]
    fn green_wraps_text_with_foreground_codes() {
        assert_eq!("ok".green().to_string(), "\x1B[32mok\x1B[39m");
    }

    #[test]
    fn chained_styles_nest_in_call_order() {
        assert_eq!(
            "ok".green().bold().to_string(),
            "\x1B[1m\x1B[32mok\x1B[39m\x1B[22m"
        );
    }
}
