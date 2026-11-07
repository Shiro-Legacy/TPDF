//! Smart defaults for the question list (design form, phase 2 B), all local and plain
//! rules: the answer type a question's words suggest, a field name from the question,
//! the choice sets one click fills in, and choices pasted as several lines.

/// One click's choices.
pub const PRESETS: [(&str, &str, &[&str]); 7] = [
    ("yes-no", "Yes / No", &["Yes", "No"]),
    (
        "yes-no-unsure",
        "Yes / No / Not sure",
        &["Yes", "No", "Not sure"],
    ),
    (
        "agree",
        "Agree scale",
        &[
            "Strongly disagree",
            "Disagree",
            "Neutral",
            "Agree",
            "Strongly agree",
        ],
    ),
    ("1-5", "1 to 5", &["1", "2", "3", "4", "5"]),
    (
        "1-10",
        "1 to 10",
        &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
    ),
    (
        "0-10",
        "0 to 10 (how likely to recommend)",
        &["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
    ),
    (
        "satisfaction",
        "Satisfaction",
        &[
            "Very dissatisfied",
            "Dissatisfied",
            "Neutral",
            "Satisfied",
            "Very satisfied",
        ],
    ),
];

/// The choices of a preset by its key.
pub fn preset(key: &str) -> Option<Vec<String>> {
    PRESETS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, _, c)| c.iter().map(|s| s.to_string()).collect())
}

/// A guess: the type, and the choices it comes with (empty for none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guess {
    pub kind: &'static str,
    pub choices: Vec<String>,
}

fn guess(kind: &'static str) -> Option<Guess> {
    Some(Guess {
        kind,
        choices: Vec::new(),
    })
}

fn with(kind: &'static str, preset_key: &str) -> Option<Guess> {
    Some(Guess {
        kind,
        choices: preset(preset_key).unwrap_or_default(),
    })
}

/// Whether `text` holds `word` as a whole word (or words).
fn has(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices(word).any(|(start, _)| {
        let end = start + word.len();
        let before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        before && after
    })
}

/// The answer type the question's words suggest, if any. The first rule that matches
/// wins: specific words (email, date, phone, signature) before the shape of the
/// sentence ("Do you …?").
pub fn guess_type(question: &str) -> Option<Guess> {
    let q = question.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    let any = |words: &[&str]| words.iter().any(|w| has(&q, w));
    let starts = |words: &[&str]| words.iter().any(|w| q.starts_with(w));
    if any(&["signature", "sign here", "signed by", "your sign"]) {
        return guess("signature");
    }
    if starts(&[
        "i agree",
        "i confirm",
        "i accept",
        "i consent",
        "i understand",
        "i certify",
    ]) {
        return guess("agree");
    }
    if any(&["email", "e-mail", "email address"]) {
        return guess("email");
    }
    if any(&["phone", "telephone", "mobile", "cell", "tel"]) {
        return guess("phone");
    }
    if any(&[
        "date",
        "dob",
        "birthday",
        "born",
        "when did",
        "when will",
        "when were",
    ]) {
        return guess("date");
    }
    if any(&["how satisfied", "satisfaction"]) {
        return with("one", "satisfaction");
    }
    // Several statements to agree with: a grid.
    if any(&["agree", "disagree"])
        && any(&[
            "the following",
            "these statements",
            "each statement",
            "statements",
        ])
    {
        return with("likert", "agree");
    }
    if any(&["agree", "disagree"]) && !starts(&["do you", "did you"]) {
        return with("one", "agree");
    }
    if any(&["how likely", "recommend"]) {
        return with("one", "0-10");
    }
    if any(&[
        "rate",
        "rating",
        "on a scale",
        "stars",
        "out of 5",
        "out of five",
    ]) {
        return guess("rating");
    }
    if any(&[
        "how many",
        "how much",
        "number of",
        "age",
        "amount",
        "quantity",
        "how old",
    ]) {
        return guess("number");
    }
    if any(&[
        "describe",
        "tell us",
        "explain",
        "comments",
        "comment",
        "feedback",
        "anything else",
        "address",
        "why",
        "details",
    ]) {
        return guess("paragraph");
    }
    let yes_no = [
        "do you",
        "did you",
        "are you",
        "were you",
        "have you",
        "has your",
        "will you",
        "would you",
        "can you",
        "could you",
        "is your",
        "is there",
        "is this",
        "does your",
        "should we",
        "may we",
        "shall we",
    ];
    if starts(&yes_no) && q.ends_with('?') {
        return with("yesno", "yes-no");
    }
    if starts(&["which", "choose", "select", "pick"]) {
        return Some(Guess {
            kind: "one",
            choices: vec!["Choice 1".into(), "Choice 2".into()],
        });
    }
    None
}

/// A field name from the question: its key words, the asking words dropped
/// ("What is your email address?" → "Email address"), at most 40 characters and cut
/// between words.
pub fn name_from_question(question: &str) -> String {
    let mut q = question
        .trim()
        .trim_end_matches(|c: char| matches!(c, '?' | ':' | '.' | '!' | '*') || c.is_whitespace())
        .to_string();
    let lower = q.to_lowercase();
    const LEADS: [&str; 22] = [
        "what is your ",
        "what's your ",
        "what is the ",
        "what are your ",
        "what is ",
        "what are ",
        "please enter your ",
        "please enter ",
        "please give your ",
        "please provide your ",
        "please tell us your ",
        "please list your ",
        "please write your ",
        "please ",
        "enter your ",
        "give your ",
        "provide your ",
        "tell us your ",
        "your ",
        "type your ",
        "write your ",
        "the ",
    ];
    if let Some(lead) = LEADS.iter().find(|l| lower.starts_with(*l))
        && q.len() > lead.len()
    {
        q = q[lead.len()..].to_string();
    }
    let mut out = String::new();
    for word in q.split_whitespace() {
        let next = if out.is_empty() {
            word.to_string()
        } else {
            format!("{out} {word}")
        };
        if next.chars().count() > 40 {
            if out.is_empty() {
                out = next.chars().take(40).collect();
            }
            break;
        }
        out = next;
    }
    let mut chars = out.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Choices pasted as several lines: one each, blank lines and bullets dropped.
pub fn split_pasted_choices(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(['-', '*', '•', '·'])
                .trim()
                .to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// The date field's help line for a locale (month first in the US and a few others).
pub fn date_help(locale: &str) -> &'static str {
    let region = locale
        .split(['-', '_'])
        .nth(1)
        .unwrap_or("")
        .to_ascii_uppercase();
    if matches!(region.as_str(), "US" | "PH" | "FM" | "MH" | "PW") {
        "Month / day / year"
    } else {
        "Day / month / year"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(q: &str) -> Option<&'static str> {
        guess_type(q).map(|g| g.kind)
    }

    #[test]
    fn the_words_suggest_the_type() {
        assert_eq!(kind("What is your email address?"), Some("email"));
        assert_eq!(kind("Date of birth"), Some("date"));
        assert_eq!(kind("DOB"), Some("date"));
        assert_eq!(kind("Mobile number"), Some("phone"));
        assert_eq!(kind("How many people are coming?"), Some("number"));
        assert_eq!(kind("Age"), Some("number"));
        assert_eq!(kind("Do you have a car?"), Some("yesno"));
        assert_eq!(kind("Are you over 18?"), Some("yesno"));
        assert_eq!(kind("Tell us about yourself"), Some("paragraph"));
        assert_eq!(kind("Home address"), Some("paragraph"));
        assert_eq!(kind("I agree to the terms"), Some("agree"));
        assert_eq!(kind("Signature"), Some("signature"));
        assert_eq!(kind("Which colour do you like?"), Some("one"));
        assert_eq!(kind("Name"), None);
        assert_eq!(kind(""), None);
        // A word inside another word is not that word.
        assert_eq!(kind("Template name"), None);
        assert_eq!(kind("Category"), None);
    }

    #[test]
    fn scales_come_with_their_choices() {
        let g = guess_type("How satisfied are you with the service?").unwrap();
        assert_eq!((g.kind, g.choices.len()), ("one", 5));
        assert_eq!(g.choices[0], "Very dissatisfied");
        let g = guess_type("How likely are you to recommend us?").unwrap();
        assert_eq!(g.choices.first().map(String::as_str), Some("0"));
        assert_eq!(g.choices.len(), 11);
        assert_eq!(kind("Please rate the food"), Some("rating"));
        assert_eq!(kind("How many stars would you give us?"), Some("rating"));
        assert_eq!(kind("Stars for the venue"), Some("rating"));
        let g = guess_type("How much do you agree with the following?").unwrap();
        assert_eq!(g.kind, "likert");
        assert_eq!(g.choices[4], "Strongly agree");
        let g = guess_type("The staff were helpful: agree or disagree").unwrap();
        assert_eq!((g.kind, g.choices.len()), ("one", 5));
        assert_eq!(guess_type("Do you drive?").unwrap().choices, ["Yes", "No"]);
    }

    #[test]
    fn names_keep_the_key_words() {
        assert_eq!(
            name_from_question("What is your email address?"),
            "Email address"
        );
        assert_eq!(
            name_from_question("Please enter your full name:"),
            "Full name"
        );
        assert_eq!(name_from_question("Do you drive?"), "Do you drive");
        assert_eq!(
            name_from_question("Tell us about the most interesting place you have ever visited"),
            "Tell us about the most interesting place"
        );
        assert_eq!(name_from_question("   "), "");
    }

    #[test]
    fn pasted_lines_are_choices_and_dates_follow_the_locale() {
        assert_eq!(
            split_pasted_choices("- Red\n\n• Green \n  Blue"),
            ["Red", "Green", "Blue"]
        );
        assert_eq!(date_help("en-US"), "Month / day / year");
        assert_eq!(date_help("en_GB"), "Day / month / year");
        assert_eq!(date_help("fr"), "Day / month / year");
        assert_eq!(preset("yes-no").unwrap(), ["Yes", "No"]);
        assert_eq!(preset("nope"), None);
    }
}
