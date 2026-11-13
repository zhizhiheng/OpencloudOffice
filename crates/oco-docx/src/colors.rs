//! WordprocessingML theme colours. Resolve after style inheritance so an
//! explicit RGB value also replaces an inherited theme reference.
use crate::values::Color;
use crate::xml::w_attr;
use crate::xmltree::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub enum ThemeSlot {
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
    Text1,
    Text2,
    Background1,
    Background2,
}

impl ThemeSlot {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "dk1" | "dark1" => Self::Dark1,
            "lt1" | "light1" => Self::Light1,
            "dk2" | "dark2" => Self::Dark2,
            "lt2" | "light2" => Self::Light2,
            "accent1" => Self::Accent1,
            "accent2" => Self::Accent2,
            "accent3" => Self::Accent3,
            "accent4" => Self::Accent4,
            "accent5" => Self::Accent5,
            "accent6" => Self::Accent6,
            "hlink" | "hyperlink" => Self::Hyperlink,
            "folHlink" | "followedHyperlink" => Self::FollowedHyperlink,
            "t1" | "text1" => Self::Text1,
            "t2" | "text2" => Self::Text2,
            "bg1" | "background1" => Self::Background1,
            "bg2" | "background2" => Self::Background2,
            _ => return None,
        })
    }

    pub(crate) fn default_colour(self) -> Self {
        match self {
            Self::Text1 => Self::Dark1,
            Self::Text2 => Self::Dark2,
            Self::Background1 => Self::Light1,
            Self::Background2 => Self::Light2,
            slot => slot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThemeColor {
    pub slot: ThemeSlot,
    pub tint: Option<u8>,
    pub shade: Option<u8>,
    /// Word's saved RGB result; already tinted/shaded, so do not transform it twice.
    pub fallback: Option<[u8; 3]>,
}

impl Color {
    /// `value` is val for text, color for a rule, or fill for shading.
    pub(crate) fn read(node: Node, value: &str) -> Option<Self> {
        let fallback = w_attr(node, value).and_then(Self::parse);
        let fill = value == "fill";
        let slot = w_attr(node, if fill { "themeFill" } else { "themeColor" }).and_then(ThemeSlot::parse);
        let Some(slot) = slot else { return fallback };
        let byte = |name| w_attr(node, name).filter(|s| s.len() == 2).and_then(|s| u8::from_str_radix(s, 16).ok());
        Some(Self::Theme(ThemeColor {
            slot,
            tint: byte(if fill { "themeFillTint" } else { "themeTint" }),
            shade: byte(if fill { "themeFillShade" } else { "themeShade" }),
            fallback: match fallback {
                Some(Self::Rgb(r, g, b)) => Some([r, g, b]),
                _ => None,
            },
        }))
    }
}

/// Adjust HSL luminance, preserving hue and saturation (ISO/IEC 29500-1,
/// 17.3.2.6). The byte is the fraction of the original luminance retained.
pub(crate) fn luminance(rgb: [u8; 3], tint: Option<u8>, shade: Option<u8>) -> [u8; 3] {
    if (tint.is_none() && shade.is_none()) || tint == Some(255) || (tint.is_none() && shade == Some(255)) {
        return rgb;
    }
    let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let original_light = (max + min) / 2.0;
    // The 240-step HLS conversion reproduces Word's stored heading colours
    // (365F91 / 243F60) and Microsoft's 943634 / 95B3D7 examples. Working
    // solely in floating-point HSL shifts those channels by one or two.
    let light = (original_light * 240.0).floor() / 240.0;
    let adjusted = match (tint, shade) {
        (Some(tint), _) => 1.0 - (1.0 - light) * f64::from(tint) / 255.0,
        (_, Some(shade)) => light * f64::from(shade) / 255.0,
        _ => light,
    };
    let delta = max - min;
    if delta < f64::EPSILON {
        return [(adjusted * 255.0 + 1e-8).floor() as u8; 3];
    }
    let saturation = ((delta / (1.0 - (2.0 * original_light - 1.0).abs())) * 240.0).round() / 240.0;
    let hue = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    let hue = (hue * 40.0).round() / 40.0;
    let chroma = (1.0 - (2.0 * adjusted - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - (hue.rem_euclid(2.0) - 1.0).abs());
    let channels = match hue as u8 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    channels.map(|c| ((c + adjusted - chroma / 2.0) * 255.0 + 1e-8).floor().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproduces_words_saved_heading_colours_and_published_examples() {
        // Microsoft MS-OI29500 2.1.72; and styles.xml in word-template.docx.
        assert_eq!(luminance([0xC0, 0x50, 0x4D], None, Some(0xBF)), [0x94, 0x36, 0x34]);
        assert_eq!(luminance([0x4F, 0x81, 0xBD], Some(0x99), None), [0x95, 0xB3, 0xD7]);
        assert_eq!(luminance([0x4F, 0x81, 0xBD], None, Some(0xBF)), [0x36, 0x5F, 0x91]);
        assert_eq!(luminance([0x4F, 0x81, 0xBD], None, Some(0x7F)), [0x24, 0x3F, 0x60]);
        assert_eq!(luminance([0; 3], Some(0xBF), None), [0x40; 3]);
        assert_eq!(luminance([1, 2, 3], Some(255), Some(0)), [1, 2, 3]);
        assert_eq!(luminance([51, 102, 153], Some(0), None), [255; 3]);
        assert_eq!(luminance([51, 102, 153], None, Some(0)), [0; 3]);
    }
}
