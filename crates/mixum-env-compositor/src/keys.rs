//! keysym 名字 → Linux evdev keycode。
//!
//! 宿主送上来的是 keysym 名字（`"v"`、`"Return"`、`"Control_L"`），合成器要把
//! 它翻成 evdev keycode 再喂给 `KeyboardHandle`。纯查表，没有平台依赖，所以和
//! `pixels` 一样在宿主平台上也编译、也跑单测。
//!
//! 注意返回的是 **evdev 原始码**（`KEY_LEFTCTRL` = 29），不是 XKB keycode；
//! 调用方按 `Keycode::new(evdev + 8)` 转换。

/// 修饰键必须能单独收发：宿主用它们合成 `Ctrl+V`，也靠它们让容器内的
/// `Ctrl+C` 真正生效——`Key` 事件只带一个 keysym，修饰键只能自己走一趟。
pub fn to_evdev(name: &str) -> Option<u16> {
    let evdev = match name {
        "Return" | "Enter" => 28,
        "BackSpace" => 14,
        "Tab" => 15,
        "space" => 57,
        "Escape" => 1,
        "Left" => 105,
        "Right" => 106,
        "Up" => 103,
        "Down" => 108,
        "minus" => 12,
        "equal" => 13,
        "comma" => 51,
        "period" => 52,
        "slash" => 53,
        "semicolon" => 39,
        "apostrophe" => 40,
        "Control_L" => 29,
        "Control_R" => 97,
        "Shift_L" => 42,
        "Shift_R" => 54,
        "Alt_L" => 56,
        "Alt_R" => 100,
        "Super_L" => 125,
        "Super_R" => 126,
        value if value.len() == 1 => {
            let ch = value.as_bytes()[0];
            // Linux evdev keycodes are not alphabetical.
            match ch {
                b'a' => 30,
                b'b' => 48,
                b'c' => 46,
                b'd' => 32,
                b'e' => 18,
                b'f' => 33,
                b'g' => 34,
                b'h' => 35,
                b'i' => 23,
                b'j' => 36,
                b'k' => 37,
                b'l' => 38,
                b'm' => 50,
                b'n' => 49,
                b'o' => 24,
                b'p' => 25,
                b'q' => 16,
                b'r' => 19,
                b's' => 31,
                b't' => 20,
                b'u' => 22,
                b'v' => 47,
                b'w' => 17,
                b'x' => 45,
                b'y' => 21,
                b'z' => 44,
                b'0' => 11,
                b'1' => 2,
                b'2' => 3,
                b'3' => 4,
                b'4' => 5,
                b'5' => 6,
                b'6' => 7,
                b'7' => 8,
                b'8' => 9,
                b'9' => 10,
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(evdev)
}

#[cfg(test)]
mod tests {
    use super::to_evdev;

    #[test]
    fn maps_modifiers_so_paste_can_be_synthesized() {
        // 宿主侧的 Cmd/Ctrl+V 就是靠这两个名字拼出来的。
        assert_eq!(to_evdev("Control_L"), Some(29));
        assert_eq!(to_evdev("v"), Some(47));
        assert_eq!(to_evdev("Shift_L"), Some(42));
        assert_eq!(to_evdev("Alt_L"), Some(56));
    }

    #[test]
    fn keeps_the_original_named_keys() {
        assert_eq!(to_evdev("Return"), Some(28));
        assert_eq!(to_evdev("space"), Some(57));
        assert_eq!(to_evdev("Left"), Some(105));
        assert_eq!(to_evdev("minus"), Some(12));
    }

    #[test]
    fn rejects_unknown_names() {
        assert_eq!(to_evdev("F13"), None);
        assert_eq!(to_evdev(""), None);
        assert_eq!(to_evdev("Control"), None);
    }
}
