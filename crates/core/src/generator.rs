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

impl Characters {
    /// How many characters rbw picks from: ASCII's 32 symbols, 10 digits and
    /// 52 letters, or its 24 unambiguous ones.
    fn count(self) -> u32 {
        match self {
            Characters::All => 94,
            Characters::NoSymbols => 62,
            Characters::DigitsOnly => 10,
            Characters::Unambiguous => 24,
        }
    }
}

/// The words of rbw's passphrases: the EFF's long list.
const WORDS: u32 = 7776;

/// How hard a generated password is to guess, from its options alone: each
/// character or word is drawn at random, so what it holds is what someone
/// who knows the options would have to search through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    /// Under 45 bits: found in days by an attacker with a stolen copy of a
    /// quickly hashed password.
    Weak,
    /// 45 to 60 bits.
    Fair,
    /// 60 bits or more: out of reach of guessing.
    Strong,
}

impl GeneratorOptions {
    /// The bits of randomness in what these options make.
    pub fn entropy(&self) -> f64 {
        match *self {
            GeneratorOptions::Password { length, characters } => {
                length as f64 * f64::from(characters.count()).log2()
            }
            GeneratorOptions::Passphrase { words } => words as f64 * f64::from(WORDS).log2(),
        }
    }

    pub fn strength(&self) -> Strength {
        match self.entropy() {
            bits if bits < 45. => Strength::Weak,
            bits if bits < 60. => Strength::Fair,
            _ => Strength::Strong,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password(length: usize, characters: Characters) -> GeneratorOptions {
        GeneratorOptions::Password { length, characters }
    }

    #[test]
    fn strength_follows_the_randomness() {
        assert_eq!(GeneratorOptions::default().strength(), Strength::Strong);
        assert_eq!(password(8, Characters::All).strength(), Strength::Fair);
        assert_eq!(password(10, Characters::All).strength(), Strength::Strong);
        assert_eq!(
            password(8, Characters::Unambiguous).strength(),
            Strength::Weak
        );
        assert_eq!(
            password(6, Characters::DigitsOnly).strength(),
            Strength::Weak
        );
        let words = |words| GeneratorOptions::Passphrase { words }.strength();
        assert_eq!(words(3), Strength::Weak);
        assert_eq!(words(4), Strength::Fair);
        assert_eq!(words(5), Strength::Strong);
        assert!((password(20, Characters::NoSymbols).entropy() - 119.08).abs() < 0.01);
    }
}
