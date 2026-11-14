//! List numbering (`word/numbering.xml`).
//!
//! A paragraph does not store its number. It names a numbering instance
//! (`w:num`) and a level; the instance points at an abstract definition
//! (`w:abstractNum`) holding up to nine levels, each with a number format, a
//! text pattern such as `第%1条` or `%1.%2`, and the indent and character
//! formatting of the label. The text shown is computed by counting through
//! the document, which [`Counters`] does.
//!
//! The counting rules follow Word's measured behaviour; see
//! `docs/layout.md` for sources.

use std::collections::{HashMap, HashSet};

use crate::xmltree::Node;
#[cfg(feature = "serde")]
use serde::Serialize;

use crate::coverage::Coverage;
use crate::props::{ParaProps, RunProps};
use crate::xml::{elements, int, val, w_attr, w_child, w_name};

/// What follows the label before the paragraph text begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Suffix {
    /// A tab, running to the hanging indent or the next tab stop.
    #[default]
    Tab,
    Space,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LabelAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// One level of a list definition (`w:lvl`).
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Level {
    pub start: i32,
    /// `ST_NumberFormat` value: `decimal`, `chineseCounting`, `bullet`, ...
    pub format: String,
    /// Pattern for the label; `%1` to `%9` stand for the numbers of levels 0 to 8.
    pub text: String,
    /// One-based index of the level whose use restarts this one. `None`
    /// means any shallower level; `Some(0)` means never.
    pub restart: Option<i32>,
    /// Print the numbers inherited from shallower levels as decimals.
    pub legal: bool,
    pub suffix: Suffix,
    pub align: LabelAlign,
    /// The paragraph style this level belongs to, for numbered headings.
    pub style: Option<String>,
    /// Indents (and tab stops) the level gives its paragraphs.
    pub para: ParaProps,
    /// Character formatting of the label itself.
    pub run: RunProps,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct AbstractNum {
    levels: HashMap<usize, Level>,
    /// Name of a numbering style this definition delegates to.
    num_style_link: Option<String>,
    /// Name of the numbering style this definition implements.
    style_link: Option<String>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
struct Num {
    abstract_id: i32,
    start_overrides: HashMap<usize, i32>,
    level_overrides: HashMap<usize, Level>,
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Numbering {
    abstracts: HashMap<i32, AbstractNum>,
    nums: HashMap<i32, Num>,
}

impl Numbering {
    pub(crate) fn parse(xml: &str, coverage: &mut Coverage) -> Result<Self, crate::xmltree::Error> {
        let doc = crate::xmltree::Document::parse(xml)?;
        let mut numbering = Numbering::default();
        for node in elements(doc.root_element()) {
            match w_name(node) {
                Some("abstractNum") => {
                    let Some(id) = int(w_attr(node, "abstractNumId")) else { continue };
                    let mut definition = AbstractNum::default();
                    for child in elements(node) {
                        match w_name(child) {
                            Some("lvl") => {
                                if let Some(ilvl) = level_index(child) {
                                    definition.levels.insert(ilvl, parse_level(child, coverage));
                                }
                            }
                            Some("numStyleLink") => definition.num_style_link = val(child).map(str::to_owned),
                            Some("styleLink") => definition.style_link = val(child).map(str::to_owned),
                            _ => {}
                        }
                    }
                    numbering.abstracts.insert(id, definition);
                }
                Some("num") => {
                    let Some(id) = int(w_attr(node, "numId")) else { continue };
                    let mut num = Num::default();
                    for child in elements(node) {
                        match w_name(child) {
                            Some("abstractNumId") => num.abstract_id = int(val(child)).unwrap_or(0),
                            Some("lvlOverride") => {
                                let Some(ilvl) = level_index(child) else { continue };
                                if let Some(start) = w_child(child, "startOverride").and_then(|n| int(val(n))) {
                                    num.start_overrides.insert(ilvl, start);
                                }
                                if let Some(level) = w_child(child, "lvl") {
                                    num.level_overrides.insert(ilvl, parse_level(level, coverage));
                                }
                            }
                            _ => {}
                        }
                    }
                    numbering.nums.insert(id, num);
                }
                // Picture bullets and clean-up flags.
                Some("numPicBullet" | "numIdMacAtCleanup") => {}
                _ => coverage.miss(Some("w:numbering"), node),
            }
        }
        Ok(numbering)
    }

    /// The abstract definition an instance counts in, following a
    /// `numStyleLink` to the definition that implements the style.
    fn abstract_of(&self, num_id: i32) -> Option<(i32, &AbstractNum)> {
        let id = self.nums.get(&num_id)?.abstract_id;
        let definition = self.abstracts.get(&id)?;
        if let Some(link) = &definition.num_style_link {
            let target = self.abstracts.iter().find(|(_, a)| a.style_link.as_ref() == Some(link));
            if let Some((target_id, target)) = target {
                return Some((*target_id, target));
            }
        }
        Some((id, definition))
    }

    /// The level a paragraph in list `num_id` at depth `ilvl` uses: the
    /// instance's override of the level if it has one, else the definition's.
    pub fn level(&self, num_id: i32, ilvl: usize) -> Option<&Level> {
        let num = self.nums.get(&num_id)?;
        num.level_overrides.get(&ilvl).or_else(|| self.abstract_of(num_id)?.1.levels.get(&ilvl))
    }

    /// The paragraph style that level `ilvl` of list `num_id` names with
    /// `w:pStyle`. This is read from the list's definition: an instance's
    /// override of the level does not change whose level it is.
    pub fn level_style(&self, num_id: i32, ilvl: usize) -> Option<&str> {
        self.abstract_of(num_id)?.1.levels.get(&ilvl)?.style.as_deref()
    }

    /// Whether two list instances are instances of the same definition.
    pub fn same_definition(&self, a: i32, b: i32) -> bool {
        matches!((self.abstract_of(a), self.abstract_of(b)), (Some((x, _)), Some((y, _))) if x == y)
    }

    pub fn is_empty(&self) -> bool {
        self.nums.is_empty()
    }
}

fn level_index(node: Node) -> Option<usize> {
    int(w_attr(node, "ilvl")).and_then(|i| usize::try_from(i).ok()).filter(|i| *i < 9)
}

fn parse_level(node: Node, coverage: &mut Coverage) -> Level {
    let mut level = Level { format: "decimal".to_owned(), ..Level::default() };
    for child in elements(node) {
        match w_name(child) {
            Some("start") => level.start = int(val(child)).unwrap_or(0),
            Some("numFmt") => level.format = val(child).unwrap_or("decimal").to_owned(),
            Some("lvlText") => level.text = val(child).unwrap_or_default().to_owned(),
            Some("lvlRestart") => level.restart = int(val(child)),
            Some("isLgl") => level.legal = crate::xml::on_off(child),
            Some("suff") => {
                level.suffix = match val(child) {
                    Some("space") => Suffix::Space,
                    Some("nothing") => Suffix::Nothing,
                    _ => Suffix::Tab,
                };
            }
            Some("lvlJc") => {
                level.align = match val(child) {
                    Some("center") => LabelAlign::Center,
                    Some("right" | "end") => LabelAlign::Right,
                    _ => LabelAlign::Left,
                };
            }
            Some("pStyle") => level.style = val(child).map(str::to_owned),
            Some("pPr") => level.para = ParaProps::parse(child, coverage),
            Some("rPr") => level.run = RunProps::parse(child, coverage),
            // A picture bullet, and what Word 97 would have shown.
            Some("lvlPicBulletId" | "legacy") => {}
            _ => coverage.miss(Some("w:lvl"), child),
        }
    }
    level
}

/// The label of one paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Label {
    pub text: String,
    pub num_id: i32,
    pub level: usize,
}

/// Running list counters for one story (the body, a header, a text box ...).
///
/// Call [`Counters::next`] once per numbered paragraph, in document order.
#[derive(Debug, Clone, Default)]
pub struct Counters {
    /// Current number of each level, per abstract definition. All instances
    /// of one definition share these, so a second list built on the same
    /// definition carries on counting where the first stopped.
    values: HashMap<i32, [Option<i32>; 9]>,
    /// Instances whose start override has been applied. It applies once, the
    /// first time the instance is used; that is what "restart numbering" does.
    started: HashSet<(i32, usize)>,
}

impl Counters {
    pub fn next(&mut self, numbering: &Numbering, num_id: i32, ilvl: usize) -> Option<Label> {
        let num = numbering.nums.get(&num_id)?;
        let (abstract_id, _) = numbering.abstract_of(num_id)?;
        let level = numbering.level(num_id, ilvl)?;
        let values = self.values.entry(abstract_id).or_default();

        let first_use = self.started.insert((num_id, ilvl));
        values[ilvl] = Some(match (first_use.then(|| num.start_overrides.get(&ilvl)).flatten(), values[ilvl]) {
            (Some(start), _) => *start,
            (None, Some(current)) => current + 1,
            (None, None) => level.start,
        });
        // Deeper levels start over, unless they say which level restarts them.
        for (deeper, value) in values.iter_mut().enumerate().skip(ilvl + 1) {
            let restart_after = numbering.level(num_id, deeper).and_then(|l| l.restart);
            let restarts = match restart_after {
                None => true,
                Some(0) => false,
                Some(n) => ilvl < n as usize,
            };
            if restarts {
                *value = None;
            }
        }

        let current =
            |index: usize| values[index].unwrap_or_else(|| numbering.level(num_id, index).map_or(1, |l| l.start));
        let mut text = String::new();
        let mut chars = level.text.chars().peekable();
        while let Some(ch) = chars.next() {
            let placeholder =
                (ch == '%').then(|| chars.peek().and_then(|d| d.to_digit(10))).flatten().filter(|d| *d >= 1);
            match placeholder {
                Some(digit) => {
                    chars.next();
                    let index = digit as usize - 1;
                    let own = index == ilvl;
                    let format = numbering.level(num_id, index).map_or("decimal", |l| l.format.as_str());
                    // With `isLgl` the numbers taken from shallower levels
                    // print as decimals; the level's own number keeps its format.
                    let format = if level.legal && !own { "decimal" } else { format };
                    text.push_str(&format_number(format, current(index)));
                }
                None => text.push(ch),
            }
        }
        Some(Label { text, num_id, level: ilvl })
    }
}

/// Writes `n` in a number format. Formats with a limited range fall back to
/// decimal beyond it, as Word does; unknown formats are written as decimal.
pub fn format_number(format: &str, n: i32) -> String {
    let decimal = || n.to_string();
    if n < 0 {
        return decimal();
    }
    let n_usize = n as usize;
    let from_sequence = |sequence: &str| sequence.chars().nth(n_usize.wrapping_sub(1)).map(String::from);
    let from_block = |first: u32, count: usize| {
        (1..=count).contains(&n_usize).then(|| char::from_u32(first + n as u32 - 1)).flatten().map(String::from)
    };
    match format {
        "decimal" | "decimalHalfWidth" => decimal(),
        "decimalZero" => format!("{n:02}"),
        "upperRoman" => roman(n).unwrap_or_else(decimal),
        "lowerRoman" => roman(n).map(|r| r.to_lowercase()).unwrap_or_else(decimal),
        "upperLetter" => letters(n, b'A').unwrap_or_else(decimal),
        "lowerLetter" => letters(n, b'a').unwrap_or_else(decimal),
        "ordinal" => format!("{n}{}", ordinal_suffix(n)),
        "cardinalText" => english(n, false).unwrap_or_else(decimal),
        "ordinalText" => english(n, true).unwrap_or_else(decimal),
        "chineseCounting" | "chineseCountingThousand" | "japaneseCounting" => chinese(n, &CHINESE_PLAIN),
        "chineseLegalSimplified" => chinese(n, &CHINESE_LEGAL),
        "ideographDigital" | "japaneseDigitalTenThousand" => decimal()
            .chars()
            .map(|d| "〇一二三四五六七八九".chars().nth(d as usize - '0' as usize).unwrap_or(d))
            .collect(),
        "ideographTraditional" => from_sequence("甲乙丙丁戊己庚辛壬癸").unwrap_or_else(decimal),
        "ideographZodiac" => from_sequence("子丑寅卯辰巳午未申酉戌亥").unwrap_or_else(decimal),
        "decimalEnclosedCircle" | "decimalEnclosedCircleChinese" => from_block(0x2460, 20).unwrap_or_else(decimal),
        "decimalEnclosedParen" => from_block(0x2474, 20).unwrap_or_else(decimal),
        "decimalEnclosedFullstop" => from_block(0x2488, 20).unwrap_or_else(decimal),
        "decimalFullWidth" | "decimalFullWidth2" => {
            decimal().chars().map(|d| char::from_u32(0xFF10 + (d as u32 - '0' as u32)).unwrap_or(d)).collect()
        }
        // The marks of the Chicago Manual of Style, doubled from the fifth note on.
        "chicago" => match n_usize {
            0 => decimal(),
            _ => String::from(['*', '†', '‡', '§'][(n_usize - 1) % 4]).repeat((n_usize - 1) / 4 + 1),
        },
        "none" => String::new(),
        _ => decimal(),
    }
}

fn roman(n: i32) -> Option<String> {
    if !(1..=3999).contains(&n) {
        return None;
    }
    const PARTS: [(i32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut rest = n;
    let mut out = String::new();
    for (value, text) in PARTS {
        while rest >= value {
            out.push_str(text);
            rest -= value;
        }
    }
    Some(out)
}

/// A, B, ... Z, AA, BB, ... ZZ, AAA: the letter repeats, it does not count in base 26.
fn letters(n: i32, first: u8) -> Option<String> {
    (n >= 1).then(|| {
        let index = (n - 1) as usize;
        String::from((first + (index % 26) as u8) as char).repeat(index / 26 + 1)
    })
}

fn ordinal_suffix(n: i32) -> &'static str {
    match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    }
}

struct ChineseDigits {
    digits: [&'static str; 10],
    ten: &'static str,
    hundred: &'static str,
    thousand: &'static str,
    ten_thousand: &'static str,
    /// Write 10 to 19 as 十, 十一 ... rather than 一十, 一十一.
    bare_ten: bool,
}

const CHINESE_PLAIN: ChineseDigits = ChineseDigits {
    digits: ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
    ten: "十",
    hundred: "百",
    thousand: "千",
    ten_thousand: "万",
    bare_ten: true,
};

const CHINESE_LEGAL: ChineseDigits = ChineseDigits {
    digits: ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"],
    ten: "拾",
    hundred: "佰",
    thousand: "仟",
    ten_thousand: "万",
    bare_ten: false,
};

/// Chinese numerals as read aloud: 一百零一, 一千二百三十四, 壹拾壹.
fn chinese(n: i32, set: &ChineseDigits) -> String {
    if n == 0 {
        return set.digits[0].to_owned();
    }
    if n >= 100_000_000 {
        return n.to_string();
    }
    // Below ten thousand: thousands, hundreds, tens, units, with one 零 for
    // any run of skipped places between two written digits.
    let group = |value: i32, leading: bool| {
        let places = [(1000, set.thousand), (100, set.hundred), (10, set.ten), (1, "")];
        let mut out = String::new();
        let mut pending_zero = false;
        let mut rest = value;
        for (unit, name) in places {
            let digit = (rest / unit) as usize;
            rest %= unit;
            if digit == 0 {
                pending_zero = !out.is_empty() || !leading;
                continue;
            }
            if pending_zero {
                out.push_str(set.digits[0]);
                pending_zero = false;
            }
            if !(set.bare_ten && unit == 10 && digit == 1 && out.is_empty() && leading) {
                out.push_str(set.digits[digit]);
            }
            out.push_str(name);
        }
        out
    };
    let (high, low) = (n / 10_000, n % 10_000);
    let mut out = String::new();
    if high > 0 {
        out.push_str(&group(high, true));
        out.push_str(set.ten_thousand);
    }
    if low > 0 {
        out.push_str(&group(low, high == 0));
    }
    out
}

/// English number words, each word capitalised as Word writes them.
fn english(n: i32, ordinal: bool) -> Option<String> {
    const UNITS: [&str; 20] = [
        "Zero",
        "One",
        "Two",
        "Three",
        "Four",
        "Five",
        "Six",
        "Seven",
        "Eight",
        "Nine",
        "Ten",
        "Eleven",
        "Twelve",
        "Thirteen",
        "Fourteen",
        "Fifteen",
        "Sixteen",
        "Seventeen",
        "Eighteen",
        "Nineteen",
    ];
    const TENS: [&str; 10] = ["", "", "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety"];
    const ORDINAL_UNITS: [&str; 20] = [
        "Zeroth",
        "First",
        "Second",
        "Third",
        "Fourth",
        "Fifth",
        "Sixth",
        "Seventh",
        "Eighth",
        "Ninth",
        "Tenth",
        "Eleventh",
        "Twelfth",
        "Thirteenth",
        "Fourteenth",
        "Fifteenth",
        "Sixteenth",
        "Seventeenth",
        "Eighteenth",
        "Nineteenth",
    ];
    if !(0..=9999).contains(&n) {
        return None;
    }
    let n = n as usize;
    let mut words: Vec<String> = Vec::new();
    if n >= 1000 {
        words.push(format!("{} Thousand", UNITS[n / 1000]));
    }
    if n % 1000 >= 100 {
        words.push(format!("{} Hundred", UNITS[n % 1000 / 100]));
    }
    let rest = n % 100;
    if rest > 0 || n == 0 {
        words.push(if rest < 20 {
            UNITS[rest].to_owned()
        } else if rest.is_multiple_of(10) {
            TENS[rest / 10].to_owned()
        } else {
            format!("{}-{}", TENS[rest / 10], UNITS[rest % 10])
        });
    }
    let mut text = words.join(" ");
    if ordinal {
        let rest = n % 100;
        let last = if rest > 0 && rest < 20 {
            Some((UNITS[rest], ORDINAL_UNITS[rest].to_owned()))
        } else if rest >= 20 && !rest.is_multiple_of(10) {
            Some((UNITS[rest % 10], ORDINAL_UNITS[rest % 10].to_owned()))
        } else {
            None
        };
        text = match last {
            Some((cardinal, ordinal)) => format!("{}{}", &text[..text.len() - cardinal.len()], ordinal),
            None if text.ends_with('y') => format!("{}ieth", &text[..text.len() - 1]),
            None => format!("{text}th"),
        };
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(format: &str, numbers: &[i32]) -> Vec<String> {
        numbers.iter().map(|n| format_number(format, *n)).collect()
    }

    #[test]
    fn formats_chinese_numbers() {
        assert_eq!(
            all(
                "chineseCounting",
                &[1, 9, 10, 11, 20, 21, 99, 100, 101, 110, 111, 1000, 1001, 1010, 1234, 10000, 10001, 12345]
            ),
            [
                "一",
                "九",
                "十",
                "十一",
                "二十",
                "二十一",
                "九十九",
                "一百",
                "一百零一",
                "一百一十",
                "一百一十一",
                "一千",
                "一千零一",
                "一千零一十",
                "一千二百三十四",
                "一万",
                "一万零一",
                "一万二千三百四十五"
            ]
        );
        assert_eq!(
            all("chineseLegalSimplified", &[1, 10, 11, 20, 99, 100, 101, 1234]),
            ["壹", "壹拾", "壹拾壹", "贰拾", "玖拾玖", "壹佰", "壹佰零壹", "壹仟贰佰叁拾肆"]
        );
        assert_eq!(all("ideographDigital", &[1, 10, 105]), ["一", "一〇", "一〇五"]);
        assert_eq!(all("ideographTraditional", &[1, 10, 11]), ["甲", "癸", "11"]);
        assert_eq!(all("ideographZodiac", &[1, 12, 13]), ["子", "亥", "13"]);
    }

    #[test]
    fn formats_the_marks_of_footnotes() {
        // Word's "chicago": four marks, doubled from the fifth note, tripled from the ninth.
        assert_eq!(all("chicago", &[1, 2, 3, 4, 5, 8, 9, 0]), ["*", "†", "‡", "§", "**", "§§", "***", "0"]);
        assert_eq!(all("decimalEnclosedCircleChinese", &[1, 20, 21]), ["①", "⑳", "21"]);
    }

    #[test]
    fn formats_western_numbers() {
        assert_eq!(
            all("upperRoman", &[1, 4, 9, 14, 40, 99, 1999, 4000]),
            ["I", "IV", "IX", "XIV", "XL", "XCIX", "MCMXCIX", "4000"]
        );
        assert_eq!(all("lowerLetter", &[1, 26, 27, 28, 53]), ["a", "z", "aa", "bb", "aaa"]);
        assert_eq!(all("decimalZero", &[1, 9, 10, 100]), ["01", "09", "10", "100"]);
        assert_eq!(
            all("ordinal", &[1, 2, 3, 4, 11, 12, 13, 21, 102, 111]),
            ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "102nd", "111th"]
        );
        assert_eq!(
            all("cardinalText", &[1, 15, 20, 21, 100, 101, 1234]),
            [
                "One",
                "Fifteen",
                "Twenty",
                "Twenty-One",
                "One Hundred",
                "One Hundred One",
                "One Thousand Two Hundred Thirty-Four"
            ]
        );
        assert_eq!(
            all("ordinalText", &[1, 12, 20, 21, 100]),
            ["First", "Twelfth", "Twentieth", "Twenty-First", "One Hundredth"]
        );
        assert_eq!(all("decimalEnclosedCircle", &[1, 20, 21]), ["①", "⑳", "21"]);
        assert_eq!(all("decimalFullWidth", &[10]), ["１０"]);
    }
}
