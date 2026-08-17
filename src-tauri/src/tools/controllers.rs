use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct ControllerInfo {
    pub name: String,
    pub handler: String,
}

/// Lists joystick/gamepad devices currently visible to the kernel by
/// reading `/proc/bus/input/devices`. This is informational only (confirms
/// the OS sees the pad at all) — actual per-game remapping is done through
/// the `SDL_GAMECONTROLLERCONFIG` env var and the "Disable Steam Input"
/// toggle stored on each Game, since a launcher-level input remapper akin
/// to Steam Input itself is out of scope here.
pub fn list_controllers() -> Result<Vec<ControllerInfo>> {
    let content = fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
    let mut controllers = vec![];
    let mut current_name = String::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("N: Name=") {
            current_name = rest.trim_matches('"').to_string();
        } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
            if let Some(handler) = rest.split_whitespace().find(|h| h.starts_with("js")) {
                controllers.push(ControllerInfo {
                    name: if current_name.is_empty() {
                        "Unknown controller".to_string()
                    } else {
                        current_name.clone()
                    },
                    handler: handler.to_string(),
                });
            }
        }
    }
    Ok(controllers)
}

/// One captured press/movement from a physical controller, used by the
/// `SDL_GAMECONTROLLERCONFIG` mapping wizard so the user doesn't have to
/// know raw button/axis numbers by heart.
#[derive(Debug, Clone, Serialize)]
pub struct ControllerInputEvent {
    pub kind: String, // "button" | "axis" | "hat"
    pub number: u8,
    pub value: i16,
    /// Populated only for `kind == "hat"`: which hat this is (almost
    /// always `0`, the first/only one on any given pad) and which
    /// direction was pressed. SDL encodes D-Pad hats as `hH.MASK` (up=1,
    /// right=2, down=4, left=8), not as a plain button or axis — without
    /// this, a hat-reported D-Pad could only ever be captured as a raw
    /// axis, which SDL would then fail to recognize at runtime.
    #[serde(default)]
    pub hat_index: Option<u8>,
    #[serde(default)]
    pub hat_direction: Option<String>, // "up" | "down" | "left" | "right"
}

/// The `JSIOCGAXMAP` ioctl request code for the classic Linux joystick API
/// (`<linux/joystick.h>`) — the same one SDL2's own Linux joystick backend
/// and `jstest` use to read a device's raw-axis-index → `ABS_*` code table.
const JSIOCGAXMAP: libc::c_ulong = 0x80406a32;
/// `ABS_HAT0X` through `ABS_HAT3Y` from `<linux/input-event-codes.h>`: the
/// eight axis codes a D-Pad reported as a "hat" (rather than four separate
/// buttons) shows up as. They're laid out X,Y per hat, hence the `/2`,`%2`
/// math below when turning an axis index back into "which hat, X or Y".
const ABS_HAT0X: u8 = 0x10;
const ABS_HAT3Y: u8 = 0x17;

/// Reads the given joystick device's axis-index → `ABS_*` code table via
/// `JSIOCGAXMAP`, so `capture_controller_input` can tell a genuine analog
/// stick axis apart from a D-Pad reported as a "hat" — both arrive through
/// the classic joystick API as an anonymous "axis N moved" event; only this
/// ioctl reveals which raw indices are actually `ABS_HAT0X`/`ABS_HAT0Y`
/// (etc). Best-effort by design: callers should treat a failure here as
/// "no hat info available" and fall back to reporting every axis as plain
/// analog, exactly like before hat detection existed, rather than failing
/// the whole capture over it.
fn get_axis_map(handler: &str) -> Result<[u8; 64]> {
    let path = format!("/dev/input/{}", handler);
    let file = std::fs::File::open(&path).with_context(|| format!("Cannot open {}", path))?;
    let mut axmap = [0u8; 64];
    let ret = unsafe { libc::ioctl(file.as_raw_fd(), JSIOCGAXMAP, axmap.as_mut_ptr()) };
    if ret < 0 {
        bail!(std::io::Error::last_os_error());
    }
    Ok(axmap)
}

/// Blocks (in a background thread, joined with a timeout) until the given
/// joystick device reports a real button press or a significant axis
/// movement, then returns which one. Used to build an `SDL_GAMECONTROLLERCONFIG`
/// entry ("press the button you want to use for A") without requiring the
/// user to already know the Linux joystick numbering for their pad.
///
/// Implementation note: this reads the classic Linux joystick API
/// (`/dev/input/jsN`, 8-byte `js_event` records) directly rather than
/// depending on SDL2 itself. Startup "init" events (which replay the
/// current state of every axis/button when the device is opened) are
/// filtered out. Axis movements are cross-checked against `get_axis_map`
/// so a D-Pad reported as a hat is captured (and encoded) correctly
/// instead of always coming back as a generic axis. If nothing arrives
/// before the timeout, the reader thread is left blocked on the device and
/// is cleaned up whenever the next input event on that device eventually
/// wakes it — an accepted, bounded cost for a manual, occasionally-used
/// wizard tool rather than something that runs continuously.
pub fn capture_controller_input(handler: &str, timeout_ms: u64) -> Result<ControllerInputEvent> {
    let path = format!("/dev/input/{}", handler);
    let mut file =
        std::fs::File::open(&path).with_context(|| format!("Cannot open {}", path))?;

    // Best-effort: if this fails, `axis_map` stays all-zero, and every axis
    // is reported as plain "axis" — the exact behavior this had before hat
    // detection was added.
    let axis_map = get_axis_map(handler).unwrap_or([0u8; 64]);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8];
        loop {
            if file.read_exact(&mut buf).is_err() {
                break;
            }
            let value = i16::from_le_bytes([buf[4], buf[5]]);
            let kind_byte = buf[6];
            let number = buf[7];
            let is_init = kind_byte & 0x80 != 0;
            let kind = kind_byte & !0x80;
            if is_init {
                continue;
            }
            if kind == 0x01 && value == 1 {
                let _ = tx.send(ControllerInputEvent {
                    kind: "button".to_string(),
                    number,
                    value,
                    hat_index: None,
                    hat_direction: None,
                });
                break;
            } else if kind == 0x02 && value.unsigned_abs() > 16000 {
                let abs_code = axis_map.get(number as usize).copied().unwrap_or(0);
                let event = if (ABS_HAT0X..=ABS_HAT3Y).contains(&abs_code) {
                    let hat_index = (abs_code - ABS_HAT0X) / 2;
                    let is_x_axis = (abs_code - ABS_HAT0X) % 2 == 0;
                    let direction = match (is_x_axis, value < 0) {
                        (true, true) => "left",
                        (true, false) => "right",
                        (false, true) => "up",
                        (false, false) => "down",
                    };
                    ControllerInputEvent {
                        kind: "hat".to_string(),
                        number,
                        value,
                        hat_index: Some(hat_index),
                        hat_direction: Some(direction.to_string()),
                    }
                } else {
                    ControllerInputEvent {
                        kind: "axis".to_string(),
                        number,
                        value,
                        hat_index: None,
                        hat_direction: None,
                    }
                };
                let _ = tx.send(event);
                break;
            }
        }
    });

    rx.recv_timeout(Duration::from_millis(timeout_ms)).map_err(|_| {
        anyhow::anyhow!(
            "No input detected within {}ms — press a button or move a stick/trigger on the \
             controller and try again.",
            timeout_ms
        )
    })
}

fn read_hex_u16(path: &str) -> Result<u16> {
    let s = fs::read_to_string(path).with_context(|| format!("Cannot read {}", path))?;
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16)
        .with_context(|| format!("Unexpected content in {}", path))
}

/// Reconstructs the SDL2-style joystick GUID for a Linux `jsN` device from
/// its USB/Bluetooth identity in sysfs (bustype/vendor/product/version),
/// using the same 16-byte little-endian layout SDL uses on Linux. This is
/// what `SDL_GAMECONTROLLERCONFIG` entries are keyed on — get it wrong and
/// the custom mapping simply won't match the pad at runtime, so if this
/// can't be read, the wizard should let the user fall back to editing the
/// GUID field manually (it's exposed as an editable field, not baked in).
pub fn get_controller_guid(handler: &str) -> Result<String> {
    let base = format!("/sys/class/input/{}/device/id", handler);
    let bustype = read_hex_u16(&format!("{}/bustype", base))?;
    let vendor = read_hex_u16(&format!("{}/vendor", base))?;
    let product = read_hex_u16(&format!("{}/product", base))?;
    let version = read_hex_u16(&format!("{}/version", base))?;

    let mut bytes = [0u8; 16];
    bytes[0..2].copy_from_slice(&bustype.to_le_bytes());
    bytes[4..6].copy_from_slice(&vendor.to_le_bytes());
    bytes[8..10].copy_from_slice(&product.to_le_bytes());
    bytes[12..14].copy_from_slice(&version.to_le_bytes());
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}
