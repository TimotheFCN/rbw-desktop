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
    /// The classes chosen one by one, as in Bitwarden's generator. `All`,
    /// `NoSymbols` and `DigitsOnly` are the same as their classes without
    /// `each_class`.
    Classes(CharacterClasses),
}

/// The classes of characters a password is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterClasses {
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    /// ASCII's 32 symbols.
    pub symbols: bool,
    /// At least one character of each class chosen. The password is drawn
    /// uniformly from all those that hold every class, so it loses only the
    /// randomness of the ones that don't: under a bit from 10 characters
    /// on, with four classes.
    pub each_class: bool,
}

const UPPERCASE: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWERCASE: &str = "abcdefghijklmnopqrstuvwxyz";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
const UNAMBIGUOUS: &str = "34678abcdefhjkmnpqrtuwxy";

impl Characters {
    /// The characters rbw picks from, by class.
    pub(crate) fn classes(self) -> Vec<&'static str> {
        let classes = match self {
            Characters::All => CharacterClasses::ALL,
            Characters::NoSymbols => CharacterClasses {
                symbols: false,
                ..CharacterClasses::ALL
            },
            Characters::DigitsOnly => CharacterClasses {
                uppercase: false,
                lowercase: false,
                symbols: false,
                ..CharacterClasses::ALL
            },
            Characters::Unambiguous => return vec![UNAMBIGUOUS],
            Characters::Classes(classes) => classes,
        };
        [
            (classes.uppercase, UPPERCASE),
            (classes.lowercase, LOWERCASE),
            (classes.digits, DIGITS),
            (classes.symbols, SYMBOLS),
        ]
        .into_iter()
        .filter_map(|(chosen, class)| chosen.then_some(class))
        .collect()
    }

    pub(crate) fn each_class(self) -> bool {
        matches!(self, Characters::Classes(classes) if classes.each_class)
    }
}

impl CharacterClasses {
    const ALL: Self = Self {
        uppercase: true,
        lowercase: true,
        digits: true,
        symbols: true,
        each_class: false,
    };
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
    /// The bits of randomness in what these options make: the logarithm of
    /// how many values they can make, all equally likely. 0 when they make
    /// nothing.
    pub fn entropy(&self) -> f64 {
        if !self.makes_anything() {
            return 0.;
        }
        match *self {
            GeneratorOptions::Password { length, characters } => {
                password_entropy(length, characters)
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

    /// Whether these options make anything: a password needs a length and
    /// characters, and room for one of each class when it must hold them.
    pub fn makes_anything(&self) -> bool {
        match *self {
            GeneratorOptions::Password { length, characters } => {
                let classes = characters.classes().len();
                length > 0 && classes > 0 && (!characters.each_class() || length >= classes)
            }
            GeneratorOptions::Passphrase { words } => words > 0,
        }
    }
}

fn password_entropy(length: usize, characters: Characters) -> f64 {
    let sizes: Vec<f64> = characters
        .classes()
        .iter()
        .map(|class| class.len() as f64)
        .collect();
    let all: f64 = sizes.iter().sum();
    let bits = length as f64 * all.log2();
    if !characters.each_class() {
        return bits;
    }
    // the share of passwords that hold every class: by inclusion-exclusion
    // over the classes left out, each term the share of passwords without
    // them
    let length = i32::try_from(length).unwrap_or(i32::MAX);
    let share: f64 = (0..1usize << sizes.len())
        .map(|left_out| {
            let missing: f64 = (0..sizes.len())
                .filter(|class| left_out & (1 << class) != 0)
                .map(|class| sizes[class])
                .sum();
            let term = (1. - missing / all).powi(length);
            if left_out.count_ones() % 2 == 0 {
                term
            } else {
                -term
            }
        })
        .sum();
    bits + share.log2()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password(length: usize, characters: Characters) -> GeneratorOptions {
        GeneratorOptions::Password { length, characters }
    }

    fn classes(digits: bool, symbols: bool, each_class: bool) -> Characters {
        Characters::Classes(CharacterClasses {
            uppercase: true,
            lowercase: true,
            digits,
            symbols,
            each_class,
        })
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

    #[test]
    fn the_shorthands_are_their_classes() {
        for (shorthand, digits, symbols) in [
            (Characters::All, true, true),
            (Characters::NoSymbols, true, false),
        ] {
            assert_eq!(
                shorthand.classes(),
                classes(digits, symbols, false).classes()
            );
        }
        assert_eq!(Characters::DigitsOnly.classes(), vec![DIGITS]);
        assert_eq!(classes(false, false, false).classes().concat().len(), 52);
        assert_eq!(Characters::All.classes().concat().len(), 94);
    }

    // The number of passwords of a length over the classes that hold each
    // of them, counted exactly: one by one, over growing lengths, by how
    // many of the classes they hold so far.
    fn count_holding_each(sizes: &[u128], length: usize) -> u128 {
        let mut by_held = std::collections::HashMap::from([(0usize, 1u128)]);
        for _ in 0..length {
            let mut next = std::collections::HashMap::new();
            for (held, count) in by_held {
                for (class, size) in sizes.iter().enumerate() {
                    *next.entry(held | 1 << class).or_insert(0) += count * size;
                }
            }
            by_held = next;
        }
        by_held.get(&((1 << sizes.len()) - 1)).copied().unwrap_or(0)
    }

    #[test]
    fn requiring_each_class_counts_only_the_passwords_that_hold_them() {
        for (characters, sizes) in [
            (classes(true, true, true), &[26u128, 26, 10, 32][..]),
            (classes(true, false, true), &[26, 26, 10][..]),
            (classes(false, false, true), &[26, 26][..]),
        ] {
            for length in sizes.len()..=18 {
                let exact = (count_holding_each(sizes, length) as f64).log2();
                let entropy = password(length, characters).entropy();
                assert!(
                    (entropy - exact).abs() < 1e-9,
                    "{characters:?} {length}: {entropy} != {exact}"
                );
            }
        }
        let each = password(20, classes(true, true, true)).entropy();
        let any = password(20, Characters::All).entropy();
        assert!(any - each > 0.1 && any - each < 0.2, "{any} {each}");
    }

    #[test]
    fn options_that_make_nothing_have_no_randomness() {
        let none = Characters::Classes(CharacterClasses {
            uppercase: false,
            lowercase: false,
            digits: false,
            symbols: false,
            each_class: false,
        });
        for options in [
            password(0, Characters::All),
            password(12, none),
            password(3, classes(true, true, true)),
            GeneratorOptions::Passphrase { words: 0 },
        ] {
            assert!(!options.makes_anything(), "{options:?}");
            assert_eq!(options.entropy(), 0.);
        }
        assert!(password(4, classes(true, true, true)).makes_anything());
    }
}
