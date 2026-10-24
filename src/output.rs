//! Pure serializers. Generated Lua returns an alpha-nvim header section.
use crate::Art;
use std::fmt::Write;
#[derive(Clone, Copy, Debug, clap::ValueEnum, PartialEq, Eq)]
pub enum Format {
    Text,
    Ansi,
    Lua,
    Html,
    Json,
}
pub fn render(art: &Art, format: Format, color: bool) -> String {
    if format == Format::Json {
        return serde_json::to_string(art).expect("finite diagnostics");
    }
    let mut s = String::new();
    render_into(art, format, color, &mut s);
    s
}
/// Clear and replace the output. Text/ANSI/HTML/Lua retain the caller's string capacity.
/// JSON delegates to serde_json and replaces the allocation; use to_writer for JSON streams.
pub fn render_into(art: &Art, format: Format, color: bool, s: &mut String) {
    if format == Format::Json {
        *s = serde_json::to_string(art).expect("finite diagnostics");
        return;
    }
    s.clear();
    s.reserve(art.cells.len().saturating_mul(if color { 24 } else { 4 }));
    if format == Format::Lua {
        lua_into(art, color, s);
        return;
    }
    if format == Format::Html {
        s.push_str("<!doctype html><meta charset=\"utf-8\"><pre style=\"background:#000;color:#fff;font-family:monospace\">");
    }
    for row in art.cells.chunks(art.width as usize) {
        let mut last = None;
        for cell in row {
            let glyph = if color {
                cell.glyph
            } else {
                cell.monochrome.unwrap_or(cell.glyph)
            };
            let style = (cell.rgb, cell.background);
            match format {
                Format::Ansi if color => {
                    if last != Some(style) {
                        ansi_color(s, cell.rgb, false);
                        if let Some(bg) = cell.background {
                            ansi_color(s, bg, true);
                        } else if last.is_some_and(|(_, bg)| bg.is_some()) {
                            s.push_str("\x1b[49m");
                        }
                        last = Some(style);
                    }
                    s.push(glyph);
                }
                Format::Html => {
                    if color && last != Some(style) {
                        if last.is_some() {
                            s.push_str("</span>");
                        }
                        s.push_str("<span style=\"color:#");
                        hex_color(s, cell.rgb);
                        if let Some(bg) = cell.background {
                            s.push_str(";background-color:#");
                            hex_color(s, bg);
                        }
                        s.push_str("\">");
                        last = Some(style);
                    }
                    match glyph {
                        '&' => s.push_str("&amp;"),
                        '<' => s.push_str("&lt;"),
                        '>' => s.push_str("&gt;"),
                        '"' => s.push_str("&quot;"),
                        '\'' => s.push_str("&#39;"),
                        c => s.push(c),
                    }
                }
                _ => s.push(glyph),
            }
        }
        if format == Format::Ansi && color {
            s.push_str("\x1b[0m");
        }
        if format == Format::Html && last.is_some() {
            s.push_str("</span>");
        }
        s.push('\n');
    }
    if format == Format::Html {
        s.push_str("</pre>\n");
    }
}
fn ansi_color(s: &mut String, rgb: [u8; 3], background: bool) {
    s.push_str(if background {
        "\x1b[48;2;"
    } else {
        "\x1b[38;2;"
    });
    s.push_str(DECIMAL[rgb[0] as usize]);
    s.push(';');
    s.push_str(DECIMAL[rgb[1] as usize]);
    s.push(';');
    s.push_str(DECIMAL[rgb[2] as usize]);
    s.push('m');
}
fn hex_color(s: &mut String, rgb: [u8; 3]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for value in rgb {
        s.push(HEX[(value >> 4) as usize] as char);
        s.push(HEX[(value & 15) as usize] as char);
    }
}
// All byte values have a tiny, allocation-free decimal representation.
const DECIMAL: [&str; 256] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
    "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32",
    "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46", "47", "48",
    "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61", "62", "63", "64",
    "65", "66", "67", "68", "69", "70", "71", "72", "73", "74", "75", "76", "77", "78", "79", "80",
    "81", "82", "83", "84", "85", "86", "87", "88", "89", "90", "91", "92", "93", "94", "95", "96",
    "97", "98", "99", "100", "101", "102", "103", "104", "105", "106", "107", "108", "109", "110",
    "111", "112", "113", "114", "115", "116", "117", "118", "119", "120", "121", "122", "123",
    "124", "125", "126", "127", "128", "129", "130", "131", "132", "133", "134", "135", "136",
    "137", "138", "139", "140", "141", "142", "143", "144", "145", "146", "147", "148", "149",
    "150", "151", "152", "153", "154", "155", "156", "157", "158", "159", "160", "161", "162",
    "163", "164", "165", "166", "167", "168", "169", "170", "171", "172", "173", "174", "175",
    "176", "177", "178", "179", "180", "181", "182", "183", "184", "185", "186", "187", "188",
    "189", "190", "191", "192", "193", "194", "195", "196", "197", "198", "199", "200", "201",
    "202", "203", "204", "205", "206", "207", "208", "209", "210", "211", "212", "213", "214",
    "215", "216", "217", "218", "219", "220", "221", "222", "223", "224", "225", "226", "227",
    "228", "229", "230", "231", "232", "233", "234", "235", "236", "237", "238", "239", "240",
    "241", "242", "243", "244", "245", "246", "247", "248", "249", "250", "251", "252", "253",
    "254", "255",
];
fn lua_into(art: &Art, color: bool, s: &mut String) {
    use std::collections::HashMap;
    let mut colors = HashMap::new();
    let mut ids = Vec::new();
    if color {
        ids.reserve(art.cells.len());
        let mut previous = None;
        for cell in &art.cells {
            // Numeric order matches (foreground RGB, optional background RGB).
            let style = color_key(cell.rgb) << 25
                | cell.background.map_or(0, |bg| (1 << 24) | color_key(bg));
            let next = colors.len();
            let id = match previous {
                Some((last, id)) if last == style => id,
                _ => *colors.entry(style).or_insert(next),
            };
            ids.push(id);
            previous = Some((style, id));
        }
        // IDs preserve first occurrence; definitions retain deterministic sorted order.
        s.reserve(
            colors
                .len()
                .saturating_mul(96)
                .saturating_add(art.cells.len().saturating_mul(32)),
        );
    }
    s.push_str(
        "-- Generated by CharArt.rs. Load with dofile(); assign to dashboard.section.header.\n",
    );
    let mut definitions: Vec<_> = colors.iter().collect();
    definitions.sort_unstable_by_key(|(style, _)| *style);
    for (&style, &id) in definitions {
        s.push_str("vim.api.nvim_set_hl(0, 'CharArt");
        decimal(s, id);
        s.push_str("', { fg = '#");
        hex_color(s, key_color(style >> 25));
        s.push('\'');
        if style & (1 << 24) != 0 {
            s.push_str(", bg = '#");
            hex_color(s, key_color(style));
            s.push('\'');
        }
        s.push_str(" })\n");
    }
    s.push_str("return { type = 'text', val = {\n");
    for row in art.cells.chunks(art.width as usize) {
        s.push_str("  \"");
        for cell in row {
            let glyph = if color {
                cell.glyph
            } else {
                cell.monochrome.unwrap_or(cell.glyph)
            };
            match glyph {
                '"' => s.push_str("\\\""),
                '\\' => s.push_str("\\\\"),
                c if c.is_control() => {
                    let _ = write!(s, "\\{:03}", c as u32);
                }
                c => s.push(c),
            }
        }
        s.push_str("\",\n");
    }
    if !color {
        s.push_str("}, opts = { position = 'center' } }\n");
        return;
    }
    s.push_str("}, opts = { position = 'center', hl = {\n");
    for (row, ids) in art
        .cells
        .chunks(art.width as usize)
        .zip(ids.chunks(art.width as usize))
    {
        s.push_str("  {");
        let mut byte = 0;
        let mut start = 0;
        let mut previous = None;
        for (cell, &id) in row.iter().zip(ids) {
            if previous != Some(id) {
                if let Some(id) = previous {
                    highlight(s, id, start, byte);
                }
                start = byte;
                previous = Some(id);
            }
            byte += cell.glyph.len_utf8();
        }
        if let Some(id) = previous {
            highlight(s, id, start, byte);
        }
        s.push_str("},\n");
    }
    s.push_str("} } }\n");
}
fn color_key(rgb: [u8; 3]) -> u64 {
    u64::from(rgb[0]) << 16 | u64::from(rgb[1]) << 8 | u64::from(rgb[2])
}
fn key_color(key: u64) -> [u8; 3] {
    [(key >> 16) as u8, (key >> 8) as u8, key as u8]
}
fn decimal(s: &mut String, value: usize) {
    s.push_str(itoa::Buffer::new().format(value));
}
fn highlight(s: &mut String, id: usize, start: usize, end: usize) {
    s.push_str("{'CharArt");
    decimal(s, id);
    s.push_str("',");
    decimal(s, start);
    s.push(',');
    decimal(s, end);
    s.push_str("},");
}
