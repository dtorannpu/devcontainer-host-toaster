extern crate hidapi;

use std::fmt;

const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x61;
const REPORT_SIZE: usize = 32;

const CMD_SET_VALUE: u8 = 0x07;
const CH: u8 = 3; // id_qmk_rgb_matrix_channel
const EFFECT: u8 = 2; // id_qmk_rgb_matrix_effect
const SPEED: u8 = 3;
const COLOR: u8 = 4; // id_qmk_rgb_matrix_color

const RGB_MATRIX_SOLID_COLOR: u8 = 9; // 環境によって数値が異なる場合あり

pub struct KeyboardConfig {
    pub vid: u16,
    pub pid: u16,
}

/// HIDデバイスとの実際の read/write を抽象化するトレイト。
/// テストではこれをモック実装に差し替えることで、実機を使わずに `Keyboard` のロジックを検証できる。
pub trait HidTransport {
    fn write(&self, data: &[u8]) -> Result<usize, KeyboardError>;
    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, KeyboardError>;
}

impl HidTransport for hidapi::HidDevice {
    fn write(&self, data: &[u8]) -> Result<usize, KeyboardError> {
        Ok(hidapi::HidDevice::write(self, data)?)
    }

    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> Result<usize, KeyboardError> {
        Ok(hidapi::HidDevice::read_timeout(self, buf, timeout_ms)?)
    }
}

pub struct Keyboard<D: HidTransport = hidapi::HidDevice> {
    device: D,
}

#[derive(Debug)]
pub enum KeyboardError {
    DeviceNotFound,
    Hid(hidapi::HidError),
}

impl fmt::Display for KeyboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyboardError::DeviceNotFound => write!(f, "keyboard device not found"),
            KeyboardError::Hid(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for KeyboardError {}

impl From<hidapi::HidError> for KeyboardError {
    fn from(err: hidapi::HidError) -> Self {
        KeyboardError::Hid(err)
    }
}

impl Keyboard<hidapi::HidDevice> {
    pub fn new(config: KeyboardConfig) -> Result<Self, KeyboardError> {
        let api = hidapi::HidApi::new()?;
        let device_info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == config.vid
                    && d.product_id() == config.pid
                    && d.usage_page() == USAGE_PAGE
                    && d.usage() == USAGE
            })
            .ok_or(KeyboardError::DeviceNotFound)?;
        let device = api.open_path(device_info.path())?;
        Ok(Self { device })
    }
}

impl<D: HidTransport> Keyboard<D> {
    fn send(
        &self,
        command_id: u8,
        channel_id: u8,
        value_id: u8,
        args: &[u8],
    ) -> Result<(), KeyboardError> {
        let mut report = vec![0x00, command_id, channel_id, value_id];
        report.extend_from_slice(args);
        report.resize(1 + REPORT_SIZE, 0); // 32バイト+report id分を0パディング
        self.device.write(&report)?;
        Ok(())
    }

    pub fn via_version(&self) -> Result<Option<u16>, KeyboardError> {
        const CMD_ID_GET_PROTOCOL_VERSION: u8 = 0x01; // Example command ID
        self.send(CMD_ID_GET_PROTOCOL_VERSION, 0, 0, &[])?;
        let mut buf = vec![0u8; REPORT_SIZE];
        let n = self.device.read_timeout(&mut buf, 1000)?;
        if n > 0 {
            let version = u16::from_be_bytes([buf[1], buf[2]]);
            Ok(Some(version))
        } else {
            Ok(None)
        }
    }

    pub fn error(&self) -> Result<(), KeyboardError> {
        self.send(CMD_SET_VALUE, CH, EFFECT, &[RGB_MATRIX_SOLID_COLOR])?; // まず単色モードに切り替え
        self.send(CMD_SET_VALUE, CH, SPEED, &[100])?;
        self.send(CMD_SET_VALUE, CH, COLOR, &[0, 255])?;
        Ok(())
    }

    pub fn success(&self) -> Result<(), KeyboardError> {
        self.send(CMD_SET_VALUE, CH, EFFECT, &[RGB_MATRIX_SOLID_COLOR])?; // まず単色モードに切り替え
        self.send(CMD_SET_VALUE, CH, SPEED, &[100])?;
        self.send(CMD_SET_VALUE, CH, COLOR, &[85, 255])?;
        Ok(())
    }

    pub fn confirm(&self) -> Result<(), KeyboardError> {
        self.send(CMD_SET_VALUE, CH, EFFECT, &[RGB_MATRIX_SOLID_COLOR])?; // まず単色モードに切り替え
        self.send(CMD_SET_VALUE, CH, SPEED, &[100])?;
        self.send(CMD_SET_VALUE, CH, COLOR, &[10, 255])?;
        Ok(())
    }

    pub fn off(&self) -> Result<(), KeyboardError> {
        self.send(CMD_SET_VALUE, CH, EFFECT, &[0])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockHidTransport {
        response: Vec<u8>,
        writes: RefCell<Vec<Vec<u8>>>,
    }

    impl MockHidTransport {
        fn new(response: Vec<u8>) -> Self {
            Self {
                response,
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl HidTransport for MockHidTransport {
        fn write(&self, data: &[u8]) -> Result<usize, KeyboardError> {
            self.writes.borrow_mut().push(data.to_vec());
            Ok(data.len())
        }

        fn read_timeout(&self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, KeyboardError> {
            let n = self.response.len().min(buf.len());
            buf[..n].copy_from_slice(&self.response[..n]);
            Ok(n)
        }
    }

    #[test]
    fn via_version_parses_response_payload() {
        let keyboard = Keyboard {
            device: MockHidTransport::new(vec![0x00, 0x01, 0x02]),
        };

        assert_eq!(keyboard.via_version().unwrap(), Some(0x0102));
    }

    #[test]
    fn via_version_returns_none_when_device_sends_no_response() {
        let keyboard = Keyboard {
            device: MockHidTransport::new(vec![]),
        };

        assert_eq!(keyboard.via_version().unwrap(), None);
    }

    /// `Keyboard::send` が組み立てるレポートと同じ形式(report id + コマンド + 引数 + 0パディング)を再現する。
    fn expected_report(command_id: u8, channel_id: u8, value_id: u8, args: &[u8]) -> Vec<u8> {
        let mut report = vec![0x00, command_id, channel_id, value_id];
        report.extend_from_slice(args);
        report.resize(1 + REPORT_SIZE, 0);
        report
    }

    #[test]
    fn success_sends_solid_color_effect_with_green_hue() {
        let keyboard = Keyboard {
            device: MockHidTransport::new(vec![]),
        };

        keyboard.success().unwrap();

        let writes = keyboard.device.writes.borrow();
        assert_eq!(
            *writes,
            vec![
                expected_report(CMD_SET_VALUE, CH, EFFECT, &[RGB_MATRIX_SOLID_COLOR]),
                expected_report(CMD_SET_VALUE, CH, SPEED, &[100]),
                expected_report(CMD_SET_VALUE, CH, COLOR, &[85, 255]),
            ]
        );
    }

    #[test]
    fn confirm_sends_solid_color_effect_with_blue_hue() {
        let keyboard = Keyboard {
            device: MockHidTransport::new(vec![]),
        };

        keyboard.confirm().unwrap();

        let writes = keyboard.device.writes.borrow();
        assert_eq!(
            *writes,
            vec![
                expected_report(CMD_SET_VALUE, CH, EFFECT, &[RGB_MATRIX_SOLID_COLOR]),
                expected_report(CMD_SET_VALUE, CH, SPEED, &[100]),
                expected_report(CMD_SET_VALUE, CH, COLOR, &[10, 255]),
            ]
        );
    }

    #[test]
    fn error_sends_solid_color_effect_with_red_hue() {
        let keyboard = Keyboard {
            device: MockHidTransport::new(vec![]),
        };

        keyboard.error().unwrap();

        let writes = keyboard.device.writes.borrow();
        assert_eq!(
            *writes,
            vec![
                expected_report(CMD_SET_VALUE, CH, EFFECT, &[RGB_MATRIX_SOLID_COLOR]),
                expected_report(CMD_SET_VALUE, CH, SPEED, &[100]),
                expected_report(CMD_SET_VALUE, CH, COLOR, &[0, 255]),
            ]
        );
    }

    #[test]
    fn off_disables_the_rgb_effect() {
        let keyboard = Keyboard {
            device: MockHidTransport::new(vec![]),
        };

        keyboard.off().unwrap();

        let writes = keyboard.device.writes.borrow();
        assert_eq!(
            *writes,
            vec![expected_report(CMD_SET_VALUE, CH, EFFECT, &[0])]
        );
    }
}
