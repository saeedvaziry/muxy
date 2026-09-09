use plist::{Dictionary, Value};
use std::collections::BTreeMap;
use std::io::Cursor;

pub const PLURAL_FORMAT_KEY: &str = "NSStringLocalizedFormatKey";
pub const PLURAL_SPEC_TYPE_KEY: &str = "NSStringFormatSpecTypeKey";
pub const PLURAL_VALUE_TYPE_KEY: &str = "NSStringFormatValueTypeKey";

const MAX_PLURAL_NESTING_DEPTH: usize = 8;

pub fn incompatible_key(catalog: &Dictionary) -> Option<String> {
    let mut keys = catalog.keys().collect::<Vec<_>>();
    keys.sort();
    keys.into_iter()
        .find(|key| !is_compatible(catalog.get(key), key))
        .cloned()
}

fn is_compatible(value: Option<&Value>, key: &str) -> bool {
    match value {
        Some(Value::String(translation)) => {
            if !key.contains('%') && !translation.contains('%') {
                return true;
            }
            let Some(key_format) = FormatSignature::parse(key, 1) else {
                return false;
            };
            let Some(translation_format) = FormatSignature::parse(translation, 1) else {
                return false;
            };
            translation_format.satisfies(&key_format)
        }
        Some(Value::Dictionary(entry)) => {
            let Some(key_format) = FormatSignature::parse(key, 1) else {
                return false;
            };
            is_plural_entry_compatible(entry, &key_format)
        }
        _ => false,
    }
}

fn is_plural_entry_compatible(entry: &Dictionary, key_format: &FormatSignature) -> bool {
    let Some(Value::String(format)) = entry.get(PLURAL_FORMAT_KEY) else {
        return false;
    };
    is_plural_format_compatible(format, 1, entry, key_format, 0)
}

fn is_plural_format_compatible(
    format: &str,
    first_argument_position: usize,
    entry: &Dictionary,
    key_format: &FormatSignature,
    depth: usize,
) -> bool {
    if depth > MAX_PLURAL_NESTING_DEPTH {
        return false;
    }
    let Some(signature) = FormatSignature::parse(format, first_argument_position) else {
        return false;
    };
    if !signature.satisfies(key_format) {
        return false;
    }
    for variable in signature.variables {
        let Some(Value::Dictionary(rule)) = entry.get(&variable.name) else {
            return false;
        };
        for (key, value) in rule {
            if key == PLURAL_SPEC_TYPE_KEY || key == PLURAL_VALUE_TYPE_KEY {
                if !matches!(value, Value::String(_)) {
                    return false;
                }
                continue;
            }
            let Value::String(variant) = value else {
                return false;
            };
            if !is_plural_format_compatible(
                variant,
                variable.position,
                entry,
                key_format,
                depth + 1,
            ) {
                return false;
            }
        }
    }
    true
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArgumentType {
    Object,
    CString,
    UnicharString,
    Pointer,
    Unichar,
    Int32,
    Uint32,
    Int64,
    Uint64,
    Double,
    LongDouble,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VariableReference {
    name: String,
    position: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FormatSignature {
    types_by_position: BTreeMap<usize, ArgumentType>,
    variables: Vec<VariableReference>,
}

impl FormatSignature {
    fn parse(format: &str, first_argument_position: usize) -> Option<Self> {
        if first_argument_position < 1 {
            return None;
        }
        let scalars = format.chars().collect::<Vec<_>>();
        let mut signature = Self {
            types_by_position: BTreeMap::new(),
            variables: Vec::new(),
        };
        let mut next_position = first_argument_position;
        let mut index = 0;
        while index < scalars.len() {
            if scalars[index] != '%' {
                index += 1;
                continue;
            }
            index += 1;
            if index >= scalars.len() {
                return None;
            }
            if scalars[index] == '%' {
                index += 1;
                continue;
            }
            if scalars[index] == '#' {
                let (reference, end) = read_variable(&scalars, index, next_position)?;
                next_position = next_position.max(reference.position + 1);
                signature.variables.push(reference);
                index = end;
                continue;
            }
            let specifier = read_specifier(&scalars, index, next_position)?;
            if let Some(existing) = signature.types_by_position.get(&specifier.position)
                && *existing != specifier.argument_type
            {
                return None;
            }
            signature
                .types_by_position
                .insert(specifier.position, specifier.argument_type);
            next_position = next_position.max(specifier.position + 1);
            index = specifier.end;
        }
        Some(signature)
    }

    fn satisfies(&self, other: &Self) -> bool {
        self.types_by_position
            .iter()
            .all(|(position, argument_type)| {
                other.types_by_position.get(position) == Some(argument_type)
            })
    }
}

fn read_variable(
    scalars: &[char],
    start: usize,
    position: usize,
) -> Option<(VariableReference, usize)> {
    let mut index = start + 1;
    if scalars.get(index) != Some(&'@') {
        return None;
    }
    index += 1;
    let name_start = index;
    while scalars.get(index).is_some_and(|scalar| *scalar != '@') {
        index += 1;
    }
    if index == name_start || index >= scalars.len() {
        return None;
    }
    Some((
        VariableReference {
            name: scalars[name_start..index].iter().collect(),
            position,
        },
        index + 1,
    ))
}

struct ParsedSpecifier {
    position: usize,
    argument_type: ArgumentType,
    end: usize,
}

fn read_specifier(
    scalars: &[char],
    start: usize,
    implicit_position: usize,
) -> Option<ParsedSpecifier> {
    let mut index = start;
    let digit_start = index;
    while scalars.get(index).is_some_and(char::is_ascii_digit) {
        index += 1;
    }
    let explicit_position = if index > digit_start && scalars.get(index) == Some(&'$') {
        let digits = scalars[digit_start..index].iter().collect::<String>();
        let parsed = digits.parse::<usize>().ok()?;
        if parsed < 1 {
            return None;
        }
        index += 1;
        Some(parsed)
    } else {
        index = start;
        None
    };
    while scalars
        .get(index)
        .is_some_and(|scalar| matches!(scalar, '-' | '+' | ' ' | '#' | '0' | '\''))
    {
        index += 1;
    }
    index = skip_field_size(scalars, index)?;
    if scalars.get(index) == Some(&'.') {
        index = skip_field_size(scalars, index + 1)?;
    }
    let length_start = index;
    while scalars
        .get(index)
        .is_some_and(|scalar| matches!(scalar, 'h' | 'l' | 'L' | 'q' | 'j' | 'z' | 't'))
    {
        index += 1;
    }
    let length = scalars[length_start..index].iter().collect::<String>();
    let conversion = *scalars.get(index)?;
    Some(ParsedSpecifier {
        position: explicit_position.unwrap_or(implicit_position),
        argument_type: argument_type(&length, conversion)?,
        end: index + 1,
    })
}

fn skip_field_size(scalars: &[char], start: usize) -> Option<usize> {
    if scalars.get(start) == Some(&'*') {
        return None;
    }
    let mut index = start;
    while scalars.get(index).is_some_and(char::is_ascii_digit) {
        index += 1;
    }
    Some(index)
}

fn argument_type(length: &str, conversion: char) -> Option<ArgumentType> {
    match conversion {
        '@' if length.is_empty() => Some(ArgumentType::Object),
        'd' | 'i' => signed_type(length),
        'u' | 'x' | 'X' | 'o' => unsigned_type(length),
        'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'a' | 'A' => match length {
            "" | "l" => Some(ArgumentType::Double),
            "L" => Some(ArgumentType::LongDouble),
            _ => None,
        },
        'c' if length.is_empty() => Some(ArgumentType::Int32),
        'C' if length.is_empty() => Some(ArgumentType::Unichar),
        's' if length.is_empty() => Some(ArgumentType::CString),
        'S' if length.is_empty() => Some(ArgumentType::UnicharString),
        'p' if length.is_empty() => Some(ArgumentType::Pointer),
        _ => None,
    }
}

fn signed_type(length: &str) -> Option<ArgumentType> {
    match length {
        "" | "h" | "hh" => Some(ArgumentType::Int32),
        "l" | "ll" | "q" | "j" | "z" | "t" => Some(ArgumentType::Int64),
        _ => None,
    }
}

fn unsigned_type(length: &str) -> Option<ArgumentType> {
    match length {
        "" | "h" | "hh" => Some(ArgumentType::Uint32),
        "l" | "ll" | "q" | "j" | "z" | "t" => Some(ArgumentType::Uint64),
        _ => None,
    }
}

pub fn parse_strings_catalog(data: &[u8]) -> Option<Dictionary> {
    if data.starts_with(b"<?xml") || data.starts_with(b"bplist") {
        return Value::from_reader(Cursor::new(data))
            .ok()
            .and_then(Value::into_dictionary);
    }
    StringsParser::new(std::str::from_utf8(data).ok()?).parse()
}

struct StringsParser<'a> {
    input: &'a str,
    offset: usize,
}

impl<'a> StringsParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, offset: 0 }
    }

    fn parse(mut self) -> Option<Dictionary> {
        let mut dictionary = Dictionary::new();
        loop {
            self.skip_trivia()?;
            if self.offset == self.input.len() {
                return Some(dictionary);
            }
            let key = self.parse_string()?;
            self.skip_trivia()?;
            self.consume('=')?;
            self.skip_trivia()?;
            let value = self.parse_string()?;
            self.skip_trivia()?;
            self.consume(';')?;
            dictionary.insert(key, Value::String(value));
        }
    }

    fn skip_trivia(&mut self) -> Option<()> {
        loop {
            let remainder = &self.input[self.offset..];
            let trimmed = remainder.trim_start_matches(char::is_whitespace);
            self.offset += remainder.len() - trimmed.len();
            let remainder = &self.input[self.offset..];
            if remainder.starts_with("//") {
                self.offset += remainder.find('\n').unwrap_or(remainder.len());
                continue;
            }
            if remainder.starts_with("/*") {
                let end = remainder.find("*/")?;
                self.offset += end + 2;
                continue;
            }
            return Some(());
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.input[self.offset..].starts_with('"') {
            return self.parse_quoted_string();
        }
        let start = self.offset;
        while let Some(character) = self.input[self.offset..].chars().next() {
            if character.is_whitespace() || matches!(character, '=' | ';') {
                break;
            }
            self.offset += character.len_utf8();
        }
        (self.offset > start).then(|| self.input[start..self.offset].to_owned())
    }

    fn parse_quoted_string(&mut self) -> Option<String> {
        self.consume('"')?;
        let mut output = String::new();
        loop {
            let character = self.next_char()?;
            match character {
                '"' => return Some(output),
                '\\' => {
                    let escaped = self.next_char()?;
                    match escaped {
                        'n' => output.push('\n'),
                        'r' => output.push('\r'),
                        't' => output.push('\t'),
                        '"' => output.push('"'),
                        '\\' => output.push('\\'),
                        'u' | 'U' => {
                            let digits = self.take_ascii(4)?;
                            let scalar = u32::from_str_radix(digits, 16).ok()?;
                            output.push(char::from_u32(scalar)?);
                        }
                        value => output.push(value),
                    }
                }
                value => output.push(value),
            }
        }
    }

    fn take_ascii(&mut self, count: usize) -> Option<&'a str> {
        let end = self.offset.checked_add(count)?;
        let value = self.input.get(self.offset..end)?;
        if !value.is_ascii() {
            return None;
        }
        self.offset = end;
        Some(value)
    }

    fn next_char(&mut self) -> Option<char> {
        let character = self.input[self.offset..].chars().next()?;
        self.offset += character.len_utf8();
        Some(character)
    }

    fn consume(&mut self, expected: char) -> Option<()> {
        (self.next_char()? == expected).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_catalog_parser_accepts_comments_and_escapes() {
        let catalog = parse_strings_catalog(
            br#"/* language */
            "Settings" = "Einstellungen";
            // format
            "%@ (%@)" = "%2$@ \u2013 %1$@";
            "#,
        )
        .unwrap();
        assert_eq!(catalog["Settings"].as_string(), Some("Einstellungen"));
        assert_eq!(catalog["%@ (%@)"].as_string(), Some("%2$@ – %1$@"));
    }

    #[test]
    fn catalog_format_validation_matches_swift_rules() {
        let valid = parse_strings_catalog(
            br#""Created branch %@" = "Zweig %@ erstellt";
            "%lld changes" = "%lld Aenderungen";
            "%@ (%@)" = "%2$@ - %1$@";
            "%lld%%" = "%lld%%";"#,
        )
        .unwrap();
        assert_eq!(incompatible_key(&valid), None);
        let invalid =
            parse_strings_catalog(br#""Created branch %@" = "Zweig %@ %@ erstellt";"#).unwrap();
        assert_eq!(
            incompatible_key(&invalid).as_deref(),
            Some("Created branch %@")
        );
    }

    #[test]
    fn malformed_formats_and_catalogs_are_rejected() {
        assert!(parse_strings_catalog(b"\"Settings\" = \"").is_none());
        let catalog = parse_strings_catalog(br#""value" = "%";"#).unwrap();
        assert_eq!(incompatible_key(&catalog).as_deref(), Some("value"));
    }
}
