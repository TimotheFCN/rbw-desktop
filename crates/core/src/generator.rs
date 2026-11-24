/// What [`Backend::generate`](crate::Backend::generate) makes, with the
/// choices rbw's generator offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorOptions {
    Password {
        length: usize,
        characters: Characters,
    },
    /// Words from the EFF word list, separated by spaces.
    Passphrase { words: usize },
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self::Password {
            length: 20,
            characters: Characters::All,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Characters {
    /// Letters, digits and symbols.
    All,
    /// Letters and digits.
    NoSymbols,
    DigitsOnly,
    /// Lower-case letters and digits that can't be mistaken for each other,
    /// for passwords people type or write down.
    Unambiguous,
}
