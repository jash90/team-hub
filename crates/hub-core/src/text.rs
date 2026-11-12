//! Text helpers shared by search, keys and time tracking.

/// Lower-cased text without diacritics, for search indexes and comparisons.
pub fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());

    for character in text.chars() {
        match character {
            'ą' | 'Ą' | 'á' | 'Á' | 'à' | 'À' | 'â' | 'Â' | 'ä' | 'Ä' | 'ã' | 'Ã' | 'å' | 'Å' => folded.push('a'),
            'ć' | 'Ć' | 'č' | 'Č' | 'ç' | 'Ç' => folded.push('c'),
            'ď' | 'Ď' | 'đ' | 'Đ' => folded.push('d'),
            'ę' | 'Ę' | 'é' | 'É' | 'è' | 'È' | 'ê' | 'Ê' | 'ë' | 'Ë' | 'ě' | 'Ě' => folded.push('e'),
            'í' | 'Í' | 'ì' | 'Ì' | 'î' | 'Î' | 'ï' | 'Ï' => folded.push('i'),
            'ł' | 'Ł' | 'ľ' | 'Ľ' => folded.push('l'),
            'ń' | 'Ń' | 'ñ' | 'Ñ' | 'ň' | 'Ň' => folded.push('n'),
            'ó' | 'Ó' | 'ò' | 'Ò' | 'ô' | 'Ô' | 'ö' | 'Ö' | 'õ' | 'Õ' | 'ø' | 'Ø' => folded.push('o'),
            'ř' | 'Ř' => folded.push('r'),
            'ś' | 'Ś' | 'š' | 'Š' => folded.push('s'),
            'ß' => folded.push_str("ss"),
            'ť' | 'Ť' => folded.push('t'),
            'ú' | 'Ú' | 'ù' | 'Ù' | 'û' | 'Û' | 'ü' | 'Ü' | 'ů' | 'Ů' => folded.push('u'),
            'ý' | 'Ý' => folded.push('y'),
            'ź' | 'Ź' | 'ż' | 'Ż' | 'ž' | 'Ž' => folded.push('z'),
            other => folded.extend(other.to_lowercase()),
        }
    }

    folded
}

pub fn slugify(text: &str, max_length: usize) -> String {
    let mut slug = String::new();

    for character in fold(text).chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }

    slug.truncate(max_length);

    slug.trim_end_matches('-').to_string()
}

const MINUTES_PER_HOUR: f64 = 60.0;
const HOURS_PER_DAY: f64 = 8.0;
const DAYS_PER_WEEK: f64 = 5.0;

fn unit_minutes(unit: char) -> Option<f64> {
    match unit {
        'w' => Some(DAYS_PER_WEEK * HOURS_PER_DAY * MINUTES_PER_HOUR),
        'd' => Some(HOURS_PER_DAY * MINUTES_PER_HOUR),
        'h' => Some(MINUTES_PER_HOUR),
        'm' => Some(1.0),
        _ => None,
    }
}

/// "1h 30m", "2d", "45m", "1.5h" → minutes. A day is 8 hours and a week 5 days, as in time tracking.
pub fn parse_duration(text: &str) -> Option<i64> {
    let mut minutes = 0.0;
    let mut number = String::new();
    let mut parts = 0;

    for character in text.trim().chars() {
        if character.is_ascii_digit() || character == '.' || character == ',' {
            number.push(if character == ',' { '.' } else { character });
        } else if character.is_whitespace() {
            continue;
        } else {
            let amount: f64 = number.parse().ok()?;
            minutes += amount * unit_minutes(character.to_ascii_lowercase())?;
            number.clear();
            parts += 1;
        }
    }

    if !number.is_empty() || parts == 0 {
        return None;
    }

    Some(minutes.round() as i64)
}

pub fn format_duration(minutes: i64) -> String {
    if minutes <= 0 {
        return "0m".to_string();
    }

    let mut rest = minutes;
    let mut parts = Vec::new();

    for unit in ['w', 'd', 'h', 'm'] {
        let size = unit_minutes(unit).unwrap_or(1.0) as i64;
        let amount = rest / size;

        if amount > 0 {
            parts.push(format!("{amount}{unit}"));
            rest -= amount * size;
        }
    }

    parts.join(" ")
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }

    let cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();

    format!("{}…", cut.trim_end())
}

/// The `@handle` a person is mentioned by: their name folded, with dots for spaces.
pub fn handle_of(name: &str) -> String {
    fold(name).split_whitespace().collect::<Vec<_>>().join(".")
}

/// `@handles` mentioned in a text, folded, without the `@`.
pub fn mentioned_handles(text: &str) -> Vec<String> {
    let mut handles: Vec<String> = Vec::new();
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;

    while index < characters.len() {
        let starts_word = index == 0 || !characters[index - 1].is_alphanumeric();

        if characters[index] == '@' && starts_word {
            let mut end = index + 1;

            while end < characters.len() && (characters[end].is_alphanumeric() || matches!(characters[end], '.' | '_' | '-')) {
                end += 1;
            }

            let handle = fold(&characters[index + 1..end].iter().collect::<String>());
            let handle = handle.trim_end_matches(['.', '-']).to_string();

            if !handle.is_empty() && !handles.contains(&handle) {
                handles.push(handle);
            }

            index = end;
        } else {
            index += 1;
        }
    }

    handles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_polish_letters() {
        assert_eq!(fold("Zażółć GĘŚLĄ jaźń"), "zazolc gesla jazn");
    }

    #[test]
    fn slug_is_trimmed_and_bounded() {
        assert_eq!(slugify("  Logowanie: błąd 500 (API)! ", 40), "logowanie-blad-500-api");
        assert_eq!(slugify("a very long summary indeed", 10), "a-very-lon");
    }

    #[test]
    fn durations_round_trip() {
        assert_eq!(parse_duration("1h 30m"), Some(90));
        assert_eq!(parse_duration("1,5h"), Some(90));
        assert_eq!(parse_duration("2d"), Some(960));
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(parse_duration("15"), None);
        assert_eq!(format_duration(90), "1h 30m");
        assert_eq!(format_duration(2400 + 480 + 5), "1w 1d 5m");
    }

    #[test]
    fn finds_mentions_but_not_emails() {
        assert_eq!(mentioned_handles("cc @Anna.Nowak, @jan and mail a@b.c."), vec!["anna.nowak", "jan"]);
        assert_eq!(handle_of("Żaneta Łoś"), "zaneta.los");
    }
}
