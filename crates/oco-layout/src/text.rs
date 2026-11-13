//! Character classes that drive font choice and line breaking.

/// Chinese, Japanese and Korean characters: ideographs, kana, Hangul,
/// CJK punctuation and the full-width forms.
pub fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x1100..=0x11FF | 0x2E80..=0x303F | 0x3040..=0x30FF | 0x3100..=0x312F | 0x3130..=0x318F
        | 0x3190..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF
        | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// Characters neither plain ASCII nor unambiguously East Asian: curly quotes,
/// dashes, the ellipsis, circled digits, arrows, unit signs. Which font they
/// take depends on the run (see [`slot_for`]).
fn is_ambiguous(ch: char) -> bool {
    matches!(ch as u32, 0x00A0..=0x00FF | 0x02B0..=0x04FF | 0x2000..=0x2BFF | 0xE000..=0xF8FF)
}

/// The four font slots of a run (`w:rFonts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Ascii,
    HighAnsi,
    EastAsia,
    Complex,
}

/// Which font slot Word uses for a character.
///
/// ASCII always takes the Latin font, including the digits and spaces inside
/// Chinese text. Ambiguous characters take the East Asian font only when the
/// run carries the East Asian hint, which Word adds to text typed in Chinese;
/// that is what makes a curly quote full width in one run and narrow in another.
pub fn slot_for(ch: char, east_asian_hint: bool, complex: bool) -> Slot {
    if complex {
        Slot::Complex
    } else if ch.is_ascii() {
        Slot::Ascii
    } else if is_cjk(ch) || (east_asian_hint && is_ambiguous(ch)) {
        Slot::EastAsia
    } else {
        Slot::HighAnsi
    }
}

/// May not begin a line (行首禁则). Word's default list for Simplified Chinese.
pub fn cannot_start_line(ch: char) -> bool {
    "!%),.:;>?]}¢¨°·ˇˉ―‖’”…‰′″›℃∶、。〃〉》」』】〕〗〞︶︺︾﹀﹄﹚﹜﹞！＂％＇），．：；？］｀｜｝～￠".contains(ch)
}

/// May not end a line (行尾禁则).
pub fn cannot_end_line(ch: char) -> bool {
    "$([{£¥·‘“〈《「『【〔〖〝﹙﹛﹝＄（．［｛￡￥".contains(ch)
}

/// Full-width punctuation whose ink sits in the left half of its em box,
/// leaving the right half blank: closing brackets, commas, full stops.
pub fn is_closing_punctuation(ch: char) -> bool {
    "、。，．：；？！）〉》」』】〕〗’”".contains(ch)
}

/// Full-width punctuation whose ink sits in the right half, blank on the left.
pub fn is_opening_punctuation(ch: char) -> bool {
    "（〈《「『【〔〖‘“".contains(ch)
}

/// The marks that may hang in the right margin when they do not fit: the
/// comma and full stop family only, never a bracket or a quote.
pub fn may_hang(ch: char) -> bool {
    "、。，．".contains(ch)
}

/// Part of a Latin word or number: no line break between two of these.
pub fn is_word_character(ch: char) -> bool {
    !is_cjk(ch) && !ch.is_whitespace() && (ch.is_alphanumeric() || "._,:;'\"@#$%&*+=/\\<>()[]{}~^`|!?-′″°".contains(ch))
}

/// Whether a line may be broken between `before` and `after`, two adjacent
/// characters of a paragraph. `kinsoku` applies the start- and end-of-line
/// prohibitions.
/// Stands in the text for an object in the line, such as a picture.
pub const OBJECT: char = '\u{FFFC}';

pub fn can_break_between(before: char, after: char, kinsoku: bool) -> bool {
    if before == ' ' || before == '\u{3000}' || before == '\t' {
        return after != ' ';
    }
    if after == ' ' || after == '\u{3000}' {
        return false; // spaces stay with the word before them and hang at the line end
    }
    if kinsoku && (cannot_start_line(after) || cannot_end_line(before)) {
        return false;
    }
    // A dash or an ellipsis written with two characters is never split.
    if before == after && matches!(before, '—' | '…' | '―') {
        return false;
    }
    // A picture in the line can be parted from what stands beside it.
    if before == OBJECT || after == OBJECT {
        return true;
    }
    if is_cjk(before) || is_cjk(after) {
        return true;
    }
    // Between a closing mark and an opening one, as in ”“ where one quotation
    // follows another; curly quotes are not Chinese characters by code point.
    if !before.is_ascii() && !after.is_ascii() && cannot_start_line(before) && cannot_end_line(after) {
        return true;
    }
    // Inside Western text: only after a hyphen that joins two letters.
    before == '-' && after.is_alphabetic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_font_slots() {
        assert_eq!(slot_for('A', true, false), Slot::Ascii);
        assert_eq!(slot_for('5', true, false), Slot::Ascii);
        assert_eq!(slot_for(' ', true, false), Slot::Ascii);
        assert_eq!(slot_for('中', false, false), Slot::EastAsia);
        assert_eq!(slot_for('，', false, false), Slot::EastAsia);
        assert_eq!(slot_for('“', true, false), Slot::EastAsia);
        assert_eq!(slot_for('“', false, false), Slot::HighAnsi);
        assert_eq!(slot_for('é', true, false), Slot::EastAsia);
        assert_eq!(slot_for('é', false, false), Slot::HighAnsi);
        assert_eq!(slot_for('A', false, true), Slot::Complex);
    }

    #[test]
    fn finds_break_opportunities() {
        let breaks = |text: &str| -> String {
            let chars: Vec<char> = text.chars().collect();
            let mut out = String::new();
            for (i, ch) in chars.iter().enumerate() {
                out.push(*ch);
                if i + 1 < chars.len() && can_break_between(*ch, chars[i + 1], true) {
                    out.push('|');
                }
            }
            out
        };
        assert_eq!(breaks("中文排版"), "中|文|排|版");
        assert_eq!(breaks("他说：“好。”我"), "他|说：|“好。”|我");
        assert_eq!(breaks("使用 Rust 编写"), "使|用 |Rust |编|写");
        assert_eq!(breaks("共4725个"), "共|4725|个");
        assert_eq!(breaks("（一）总则"), "（一）|总|则");
        assert_eq!(breaks("破折号——不断开……好"), "破|折|号|——|不|断|开……|好");
        assert_eq!(breaks("well-known word"), "well-|known |word");
        assert_eq!(breaks("a  b"), "a  |b");
        assert_eq!(breaks("用“一、”“（一）”标注"), "用|“一、”|“（一）”|标|注");
    }
}
