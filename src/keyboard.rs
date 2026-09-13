extern crate hidapi;

use std::fmt;

const USAGE_PAGE: u16 = 0xFF60;
const USAGE: u16 = 0x61;
const REPORT_SIZE: usize = 32;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockHidTransport {
        response: Vec<u8>,
    }

    impl HidTransport for MockHidTransport {
        fn write(&self, data: &[u8]) -> Result<usize, KeyboardError> {
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
            device: MockHidTransport {
                response: vec![0x00, 0x01, 0x02],
            },
        };

        assert_eq!(keyboard.via_version().unwrap(), Some(0x0102));
    }

    #[test]
    fn via_version_returns_none_when_device_sends_no_response() {
        let keyboard = Keyboard {
            device: MockHidTransport { response: vec![] },
        };

        assert_eq!(keyboard.via_version().unwrap(), None);
    }
}
