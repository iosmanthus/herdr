//! Tracks whether a pane's running program enabled xterm **modifyOtherKeys
//! mode 2** by observing its output byte stream, mirroring
//! [`super::kitty_keyboard::KittyKeyboardTracker`] for the Kitty keyboard
//! protocol.
//!
//! The mode is toggled by the private sequence `CSI > 4 ; Pv m`:
//!   - `Pv == 2` enables mode 2 (every modified key, including Enter/Tab, is
//!     escaped) — the only mode that makes Shift+Enter etc. meaningful to the
//!     program.
//!   - `Pv == 1`, `Pv == 0`, or a missing `Pv` (`CSI > 4 m`) turn it off for our
//!     purposes.
//!
//! Reading the control stream — rather than the terminal's replayed screen
//! state — keeps the check content-proof: a cell's text can never contain a raw
//! `ESC` byte, so pane output cannot fake the token.
#[derive(Debug, Clone, Default)]
pub(crate) struct ModifyOtherKeysTracker {
    pending: Vec<u8>,
    enabled: bool,
}

impl ModifyOtherKeysTracker {
    pub(crate) fn observe(&mut self, bytes: &[u8]) {
        let combined;
        let bytes = if self.pending.is_empty() {
            bytes
        } else {
            combined = self
                .pending
                .iter()
                .copied()
                .chain(bytes.iter().copied())
                .collect::<Vec<_>>();
            self.pending.clear();
            &combined
        };
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != 0x1b {
                index += 1;
                continue;
            }
            if index + 1 >= bytes.len() {
                self.store_pending(&bytes[index..]);
                break;
            }
            if bytes[index + 1] != b'[' {
                index += 1;
                continue;
            }

            let mut end = index + 2;
            while end < bytes.len() && !(0x40..=0x7e).contains(&bytes[end]) {
                end += 1;
            }
            if end >= bytes.len() {
                self.store_pending(&bytes[index..]);
                break;
            }

            if bytes[end] == b'm' {
                self.observe_csi_m(&bytes[index + 2..end]);
            }
            index = end + 1;
        }
    }

    fn store_pending(&mut self, bytes: &[u8]) {
        self.pending.clear();
        if bytes.len() <= 64 {
            self.pending.extend_from_slice(bytes);
        }
    }

    /// `params` are the bytes between `CSI` and the final `m` — e.g. `b">4;2"`.
    /// Only the private `CSI > 4 ; Pv m` form controls modifyOtherKeys; ordinary
    /// SGR colour sequences (no `>` marker, or a different resource) are ignored.
    fn observe_csi_m(&mut self, params: &[u8]) {
        let Some((&marker, rest)) = params.split_first() else {
            return;
        };
        if marker != b'>' {
            return;
        }
        let mut fields = rest.split(|byte| *byte == b';');
        if fields.next().and_then(parse_u16) != Some(4) {
            return;
        }
        // A missing second parameter (`CSI > 4 m`) resets the mode.
        let value = fields.next().and_then(parse_u16).unwrap_or(0);
        self.enabled = value == 2;
    }

    /// Whether the pane currently has xterm modifyOtherKeys mode 2 enabled.
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }
}

fn parse_u16(bytes: &[u8]) -> Option<u16> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_enable_and_reset() {
        let mut tracker = ModifyOtherKeysTracker::default();
        assert!(!tracker.enabled());

        tracker.observe(b"\x1b[>4;2m");
        assert!(tracker.enabled());

        // Mode 1 does not escape Enter/Tab, so it counts as "off" for us.
        tracker.observe(b"\x1b[>4;1m");
        assert!(!tracker.enabled());

        tracker.observe(b"\x1b[>4;2m");
        assert!(tracker.enabled());

        // Explicit disable and the bare reset form both turn it off.
        tracker.observe(b"\x1b[>4;0m");
        assert!(!tracker.enabled());
        tracker.observe(b"\x1b[>4;2m");
        tracker.observe(b"\x1b[>4m");
        assert!(!tracker.enabled());
    }

    #[test]
    fn ignores_ordinary_sgr_and_screen_content() {
        let mut tracker = ModifyOtherKeysTracker::default();
        tracker.observe(b"\x1b[>4;2m");
        assert!(tracker.enabled());

        // Colour SGR, cursor moves, and a prompt drawing `>` characters must not
        // flip the state — only the private `CSI > 4 ; Pv m` token may.
        tracker.observe(b"\x1b[0m\x1b[1;32muser@host \x1b[0m~ > ls\r\n");
        assert!(tracker.enabled());
    }

    #[test]
    fn buffers_split_sequences() {
        let mut tracker = ModifyOtherKeysTracker::default();
        tracker.observe(b"\x1b[>4");
        assert!(!tracker.enabled());
        tracker.observe(b";2m");
        assert!(tracker.enabled());
    }
}
