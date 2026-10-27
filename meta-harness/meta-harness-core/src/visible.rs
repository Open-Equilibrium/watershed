//! Text as Watershed shows it: characters that hide or reorder it are written out.

/// `text` with the characters that hide or reorder what it shows written out, such as
/// `<U+202E>`; [`hides`] lists them. Look-alike letters stay as they are.
pub fn visible(text: &str) -> String {
    written_out(text, hides)
}

/// A name, such as a file's or a tool's, on one line: like [`visible`], with line breaks and
/// tabs written out too.
pub fn visible_name(name: &str) -> String {
    written_out(name, |character| {
        character == '\n' || character == '\t' || hides(character)
    })
}

/// Lines as the Diff tab shows them: like [`visible`], but a carriage return that ends a line
/// stays, as Windows line ends have one, so the tab can mark it (ADR-0212).
pub fn visible_lines(text: &str) -> String {
    text.split('\n')
        .map(|line| match line.strip_suffix('\r') {
            Some(line) => visible(line) + "\r",
            None => visible(line),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `text` with each character `marked` picks written out as its code point.
fn written_out(text: &str, marked: impl Fn(char) -> bool) -> String {
    let mut shown = String::with_capacity(text.len());
    for character in text.chars() {
        if marked(character) {
            shown.push_str(&format!("<U+{:04X}>", u32::from(character)));
        } else {
            shown.push(character);
        }
    }
    shown
}

/// Whether `character` hides or reorders text, as of Unicode 16.0: control (Cc) characters
/// other than line breaks and tabs, format (Cf) characters, the separators U+2028 and U+2029,
/// and characters renderers may ignore (Default_Ignorable_Code_Point), such as variation
/// selectors and fillers.
fn hides(character: char) -> bool {
    (character.is_control() && character != '\n' && character != '\t')
        || matches!(
            u32::from(character),
            0xAD | 0x34F
                | 0x600..=0x605
                | 0x61C
                | 0x6DD
                | 0x70F
                | 0x890..=0x891
                | 0x8E2
                | 0x115F..=0x1160
                | 0x17B4..=0x17B5
                | 0x180B..=0x180F
                | 0x200B..=0x200F
                | 0x2028..=0x202E
                | 0x2060..=0x206F
                | 0x3164
                | 0xFE00..=0xFE0F
                | 0xFEFF
                | 0xFFA0
                | 0xFFF0..=0xFFFB
                | 0x110BD
                | 0x110CD
                | 0x13430..=0x1343F
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0000..=0xE0FFF
        )
}
