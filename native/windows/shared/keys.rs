//! Key translation between the wire format and Windows. The wire carries
//! macOS virtual keycodes (`kVK_*`, US ANSI positions); Windows input uses
//! set-1 scancodes, where `0xE0xx` marks an extended key. Shared by the input
//! helper, which injects and captures keys, and the display viewer, which
//! sends the keys typed into it.

/// macOS virtual keycode → Windows scancode, by physical position.
const MAC_TO_SCAN: &[(u16, u16)] = &[
    (0, 0x1E),     // A
    (1, 0x1F),     // S
    (2, 0x20),     // D
    (3, 0x21),     // F
    (4, 0x23),     // H
    (5, 0x22),     // G
    (6, 0x2C),     // Z
    (7, 0x2D),     // X
    (8, 0x2E),     // C
    (9, 0x2F),     // V
    (10, 0x56),    // ISO section, the key beside left Shift
    (11, 0x30),    // B
    (12, 0x10),    // Q
    (13, 0x11),    // W
    (14, 0x12),    // E
    (15, 0x13),    // R
    (16, 0x15),    // Y
    (17, 0x14),    // T
    (18, 0x02),    // 1
    (19, 0x03),    // 2
    (20, 0x04),    // 3
    (21, 0x05),    // 4
    (22, 0x07),    // 6
    (23, 0x06),    // 5
    (24, 0x0D),    // =
    (25, 0x0A),    // 9
    (26, 0x08),    // 7
    (27, 0x0C),    // -
    (28, 0x09),    // 8
    (29, 0x0B),    // 0
    (30, 0x1B),    // ]
    (31, 0x18),    // O
    (32, 0x16),    // U
    (33, 0x1A),    // [
    (34, 0x17),    // I
    (35, 0x19),    // P
    (36, 0x1C),    // Return
    (37, 0x26),    // L
    (38, 0x24),    // J
    (39, 0x28),    // '
    (40, 0x25),    // K
    (41, 0x27),    // ;
    (42, 0x2B),    // \
    (43, 0x33),    // ,
    (44, 0x35),    // /
    (45, 0x31),    // N
    (46, 0x32),    // M
    (47, 0x34),    // .
    (48, 0x0F),    // Tab
    (49, 0x39),    // Space
    (50, 0x29),    // `
    (51, 0x0E),    // Delete (Backspace)
    (53, 0x01),    // Escape
    (64, 0x68),    // F17
    (65, 0x53),    // Keypad .
    (67, 0x37),    // Keypad *
    (69, 0x4E),    // Keypad +
    (71, 0x45),    // Keypad Clear (Num Lock)
    (72, 0xE030),  // Volume up
    (73, 0xE02E),  // Volume down
    (74, 0xE020),  // Mute
    (75, 0xE035),  // Keypad /
    (76, 0xE01C),  // Keypad Enter
    (78, 0x4A),    // Keypad -
    (79, 0x69),    // F18
    (80, 0x6A),    // F19
    (81, 0x59),    // Keypad =
    (82, 0x52),    // Keypad 0
    (83, 0x4F),    // Keypad 1
    (84, 0x50),    // Keypad 2
    (85, 0x51),    // Keypad 3
    (86, 0x4B),    // Keypad 4
    (87, 0x4C),    // Keypad 5
    (88, 0x4D),    // Keypad 6
    (89, 0x47),    // Keypad 7
    (90, 0x6B),    // F20
    (91, 0x48),    // Keypad 8
    (92, 0x49),    // Keypad 9
    (93, 0x7D),    // JIS Yen
    (94, 0x73),    // JIS underscore (Ro)
    (95, 0x7E),    // JIS keypad comma
    (96, 0x3F),    // F5
    (97, 0x40),    // F6
    (98, 0x41),    // F7
    (99, 0x3D),    // F3
    (100, 0x42),   // F8
    (101, 0x43),   // F9
    (102, 0x7B),   // JIS Eisu (Muhenkan)
    (103, 0x57),   // F11
    (104, 0x70),   // JIS Kana
    (105, 0x64),   // F13
    (106, 0x67),   // F16
    (107, 0x65),   // F14
    (109, 0x44),   // F10
    (110, 0xE05D), // Context menu
    (111, 0x58),   // F12
    (113, 0x66),   // F15
    (114, 0xE052), // Help (Insert)
    (115, 0xE047), // Home
    (116, 0xE049), // Page up
    (117, 0xE053), // Forward delete
    (118, 0x3E),   // F4
    (119, 0xE04F), // End
    (120, 0x3C),   // F2
    (121, 0xE051), // Page down
    (122, 0x3B),   // F1
    (123, 0xE04B), // Left
    (124, 0xE04D), // Right
    (125, 0xE050), // Down
    (126, 0xE048), // Up
];

/// Modifier mask bits on the wire, left and right keys.
pub const SHIFT: u8 = 1;
pub const CONTROL: u8 = 2;
pub const OPTION: u8 = 4;
pub const COMMAND: u8 = 8;
pub const RIGHT: u8 = 0x10;

/// Scancodes for each modifier: (bit, left, right). Option is Alt and
/// Command is the Windows key, matching their keyboard positions.
pub const MODIFIERS: [(u8, u16, u16); 4] = [
    (SHIFT, 0x2A, 0x36),
    (CONTROL, 0x1D, 0xE01D),
    (OPTION, 0x38, 0xE038),
    (COMMAND, 0xE05B, 0xE05C),
];

pub fn mac_to_scan(code: u16) -> Option<u16> {
    MAC_TO_SCAN
        .iter()
        .find(|&&(mac, _)| mac == code)
        .map(|&(_, scan)| scan)
}

pub fn scan_to_mac(scan: u16) -> Option<u16> {
    MAC_TO_SCAN
        .iter()
        .find(|&&(_, s)| s == scan)
        .map(|&(mac, _)| mac)
}

/// Which modifier a scancode is, as (bit, is right-hand key).
pub fn modifier_of(scan: u16) -> Option<(u8, bool)> {
    MODIFIERS.iter().find_map(|&(bit, left, right)| {
        if scan == left {
            Some((bit, false))
        } else if scan == right {
            Some((bit, true))
        } else {
            None
        }
    })
}

/// Keys to hold for a wire modifier mask: the right-hand key when the
/// modifier's right bit is set, otherwise the left one.
pub fn modifier_keys(mask: u8) -> Vec<u16> {
    MODIFIERS
        .iter()
        .enumerate()
        .filter(|(_, (bit, _, _))| mask & bit != 0)
        .map(|(index, &(_, left, right))| {
            if mask & (RIGHT << index) != 0 {
                right
            } else {
                left
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_is_one_to_one_and_skips_modifiers() {
        for &(mac, scan) in MAC_TO_SCAN {
            assert!(
                mac < 128 && !(54..=63).contains(&mac),
                "{mac} is not a wire key"
            );
            assert_eq!(scan_to_mac(scan), Some(mac));
            assert!(modifier_of(scan).is_none());
        }
        assert_eq!(mac_to_scan(8), Some(0x2E));
        assert_eq!(mac_to_scan(56), None);
    }

    #[test]
    fn modifier_masks_choose_sides() {
        assert_eq!(modifier_keys(SHIFT | CONTROL), vec![0x2A, 0x1D]);
        // Right bits follow the modifier order: 0x10 Shift, 0x20 Control...
        assert_eq!(modifier_keys(COMMAND | (RIGHT << 3)), vec![0xE05C]);
        assert_eq!(modifier_of(0xE038), Some((OPTION, true)));
    }
}
