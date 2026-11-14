//! 会话内单实例与新窗口 IPC。协议编解码跨平台，Windows 传输只用 Win32 FFI。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

const MAGIC: &[u8; 4] = b"VEIL";
const VERSION: u8 = 1;
const KIND_NEW_WINDOW: u8 = 1;
const PAYLOAD_HEADER: usize = 8;
const LENGTH_PREFIX: usize = 4;
const MAX_PATH_BYTES: usize = 64 * 1024;
const MAX_PAYLOAD: usize = PAYLOAD_HEADER + MAX_PATH_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcRequest {
    NewWindow(Option<PathBuf>),
}

pub enum Instance {
    Primary(PrimaryInstance),
    Secondary,
}

fn validate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("IPC 文档路径必须是绝对路径".into());
    }
    let text = path
        .to_str()
        .ok_or_else(|| "IPC 文档路径不是有效 UTF-8".to_string())?;
    if text.is_empty() || text.contains('\0') {
        return Err("IPC 文档路径无效".into());
    }
    if text.len() > MAX_PATH_BYTES {
        return Err("IPC 文档路径过长".into());
    }
    Ok(())
}

pub fn encode_frame(request: &IpcRequest) -> Result<Vec<u8>, String> {
    let IpcRequest::NewWindow(path) = request;
    let path_bytes = match path {
        Some(path) => {
            validate_path(path)?;
            path.to_str().expect("路径已校验 UTF-8").as_bytes()
        }
        None => &[],
    };
    let payload_len = PAYLOAD_HEADER + path_bytes.len();
    let mut out = Vec::with_capacity(LENGTH_PREFIX + payload_len);
    out.extend_from_slice(&(payload_len as u32).to_le_bytes());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(KIND_NEW_WINDOW);
    out.push(u8::from(path.is_some()));
    out.push(0);
    out.extend_from_slice(path_bytes);
    Ok(out)
}

pub fn decode_frame(payload: &[u8]) -> Result<IpcRequest, String> {
    if payload.len() < PAYLOAD_HEADER || payload.len() > MAX_PAYLOAD {
        return Err("IPC 帧长度无效".into());
    }
    if &payload[..4] != MAGIC || payload[4] != VERSION || payload[5] != KIND_NEW_WINDOW {
        return Err("IPC 帧标识或版本无效".into());
    }
    if payload[7] != 0 || payload[6] > 1 {
        return Err("IPC 帧标志无效".into());
    }
    let has_path = payload[6] == 1;
    let path = &payload[PAYLOAD_HEADER..];
    if !has_path {
        if !path.is_empty() {
            return Err("无路径 IPC 帧包含额外数据".into());
        }
        return Ok(IpcRequest::NewWindow(None));
    }
    if path.is_empty() {
        return Err("IPC 路径为空".into());
    }
    let text = std::str::from_utf8(path).map_err(|_| "IPC 路径不是有效 UTF-8".to_string())?;
    let path = PathBuf::from(text);
    validate_path(&path)?;
    Ok(IpcRequest::NewWindow(Some(path)))
}

pub fn read_frame(reader: &mut impl Read) -> Result<IpcRequest, String> {
    let mut length = [0_u8; LENGTH_PREFIX];
    reader
        .read_exact(&mut length)
        .map_err(|e| format!("IPC 帧头截断: {e}"))?;
    let len = u32::from_le_bytes(length) as usize;
    if !(PAYLOAD_HEADER..=MAX_PAYLOAD).contains(&len) {
        return Err("IPC 帧长度超出限制".into());
    }
    let mut payload = vec![0_u8; len];
    reader
        .read_exact(&mut payload)
        .map_err(|e| format!("IPC 帧内容截断: {e}"))?;
    decode_frame(&payload)
}

pub fn write_frame(writer: &mut impl Write, request: &IpcRequest) -> Result<(), String> {
    let bytes = encode_frame(request)?;
    writer.write_all(&bytes).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

#[cfg(windows)]
mod imp {
    use super::{read_frame, write_frame, Instance, IpcRequest, PrimaryInstance};
    use std::ffi::c_void;
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read};
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::sync::mpsc::{channel, Receiver};
    use std::time::{Duration, Instant};

    type Handle = *mut c_void;
    const ERROR_ALREADY_EXISTS: u32 = 183;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const PIPE_ACCESS_INBOUND: u32 = 0x0000_0001;
    const PIPE_TYPE_BYTE: u32 = 0;
    const PIPE_READMODE_BYTE: u32 = 0;
    const PIPE_WAIT: u32 = 0;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
    /// 单个连接读完一帧的总时限：慢客户端不得长期占住唯一监听实例
    /// （DEF-T10-004）。小于第二实例的 1s 连接重试窗口。
    const READ_TIMEOUT: Duration = Duration::from_millis(500);

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(attrs: *mut c_void, initial_owner: i32, name: *const u16) -> Handle;
        fn GetLastError() -> u32;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetCurrentProcessId() -> u32;
        fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer: u32,
            in_buffer: u32,
            timeout: u32,
            attrs: *mut c_void,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
        fn DisconnectNamedPipe(pipe: Handle) -> i32;
        fn PeekNamedPipe(
            pipe: Handle,
            buffer: *mut c_void,
            buffer_size: u32,
            bytes_read: *mut u32,
            total_bytes_avail: *mut u32,
            bytes_left: *mut u32,
        ) -> i32;
    }

    /// 带总时限的管道读取器：只有 `PeekNamedPipe` 报告可读才真正 read，
    /// 否则在时限内轮询；超时返回 `TimedOut`，让监听线程安全关闭当前连接。
    struct TimedPipe {
        file: File,
        deadline: Instant,
    }

    impl Read for TimedPipe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            loop {
                let mut available = 0u32;
                // SAFETY: 句柄归 self.file 所有；输出指针均有效。
                let ok = unsafe {
                    PeekNamedPipe(
                        self.file.as_raw_handle(),
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &mut available,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                if available > 0 {
                    return self.file.read(buf);
                }
                if Instant::now() >= self.deadline {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "IPC 读帧超时"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub(super) struct Mutex(Handle);

    impl Drop for Mutex {
        fn drop(&mut self) {
            // SAFETY: 句柄来自成功的 CreateMutexW，且只在 Drop 关闭一次。
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn session_id() -> u32 {
        let mut id = 0;
        // SAFETY: 输出指针有效；失败时保留会话 0，仍由 Local 命名空间隔离。
        unsafe {
            ProcessIdToSessionId(GetCurrentProcessId(), &mut id);
        }
        id
    }

    fn mutex_name() -> String {
        format!("Local\\Veil.SingleInstance.{}", session_id())
    }

    fn pipe_name() -> String {
        format!(r"\\.\pipe\Veil.SingleInstance.{}", session_id())
    }

    pub(super) fn acquire() -> Result<Instance, String> {
        let name = super::super::wide(&mutex_name());
        // SAFETY: name 是 NUL 结尾 UTF-16；attrs 可为空。
        let handle = unsafe { CreateMutexW(std::ptr::null_mut(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error().to_string());
        }
        // SAFETY: GetLastError 无前置条件。
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: 本次取得的重复句柄不再使用。
            unsafe {
                CloseHandle(handle);
            }
            return Ok(Instance::Secondary);
        }
        Ok(Instance::Primary(PrimaryInstance {
            _mutex: Mutex(handle),
        }))
    }

    pub(super) fn listen_on(endpoint: String) -> Result<Receiver<IpcRequest>, String> {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("veil-ipc-listener".into())
            .spawn(move || loop {
                let name = super::super::wide(&endpoint);
                // SAFETY: 所有固定参数均符合 CreateNamedPipeW 字节管道约定。
                let handle = unsafe {
                    CreateNamedPipeW(
                        name.as_ptr(),
                        PIPE_ACCESS_INBOUND,
                        PIPE_TYPE_BYTE
                            | PIPE_READMODE_BYTE
                            | PIPE_WAIT
                            | PIPE_REJECT_REMOTE_CLIENTS,
                        255,
                        0,
                        4096,
                        0,
                        std::ptr::null_mut(),
                    )
                };
                if handle.is_null() || handle == -1_isize as Handle {
                    return;
                }
                // SAFETY: handle 是有效管道实例，阻塞等待一个客户端。
                let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
                // SAFETY: GetLastError 无前置条件。
                let error = unsafe { GetLastError() };
                if connected != 0 || error == ERROR_PIPE_CONNECTED {
                    // SAFETY: 所有权转交 File，作用域结束自动 CloseHandle。
                    let file = unsafe { File::from_raw_handle(handle) };
                    let mut reader = TimedPipe {
                        file,
                        deadline: Instant::now() + READ_TIMEOUT,
                    };
                    let request = read_frame(&mut reader);
                    // SAFETY: reader 仍持有句柄；断开必须在 File 关闭前执行。
                    unsafe {
                        DisconnectNamedPipe(handle);
                    }
                    if let Ok(request) = request {
                        if tx.send(request).is_err() {
                            return;
                        }
                    }
                } else {
                    // SAFETY: 未转交 File 的失败句柄仍需关闭。
                    unsafe {
                        CloseHandle(handle);
                    }
                    continue;
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(rx)
    }

    pub(super) fn listen(_mutex: &Mutex) -> Result<Receiver<IpcRequest>, String> {
        listen_on(pipe_name())
    }

    pub(super) fn send(request: &IpcRequest) -> Result<(), String> {
        send_to(pipe_name(), request)
    }

    pub(super) fn send_to(endpoint: String, request: &IpcRequest) -> Result<(), String> {
        let mut last = None;
        for attempt in 0..20 {
            match OpenOptions::new().write(true).open(&endpoint) {
                Ok(mut file) => return write_frame(&mut file, request),
                Err(e) => {
                    last = Some(e);
                    if attempt < 19 {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        }
        Err(format!(
            "无法连接 Veil 主进程: {}",
            last.map_or_else(|| "未知错误".into(), |e| e.to_string())
        ))
    }
}

#[cfg(windows)]
pub struct PrimaryInstance {
    _mutex: imp::Mutex,
}

#[cfg(windows)]
impl PrimaryInstance {
    pub fn listen(&self) -> Result<Receiver<IpcRequest>, String> {
        imp::listen(&self._mutex)
    }
}

#[cfg(windows)]
pub fn acquire() -> Result<Instance, String> {
    imp::acquire()
}

#[cfg(windows)]
pub fn send(request: &IpcRequest) -> Result<(), String> {
    imp::send(request)
}

#[cfg(not(windows))]
pub struct PrimaryInstance;

#[cfg(not(windows))]
impl PrimaryInstance {
    pub fn listen(&self) -> Result<Receiver<IpcRequest>, String> {
        let (_tx, rx) = std::sync::mpsc::channel();
        Ok(rx)
    }
}

#[cfg(not(windows))]
pub fn acquire() -> Result<Instance, String> {
    Ok(Instance::Primary(PrimaryInstance))
}

#[cfg(not(windows))]
pub fn send(_request: &IpcRequest) -> Result<(), String> {
    Err("当前平台不支持单实例 IPC；将使用独立进程".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn absolute() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"D:\docs\笔记.md")
        } else {
            PathBuf::from("/tmp/笔记.md")
        }
    }

    #[test]
    fn frame_roundtrip_with_and_without_path() {
        for request in [
            IpcRequest::NewWindow(None),
            IpcRequest::NewWindow(Some(absolute())),
        ] {
            let bytes = encode_frame(&request).unwrap();
            let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert_eq!(len, bytes.len() - LENGTH_PREFIX);
            assert_eq!(decode_frame(&bytes[4..]).unwrap(), request);
            assert_eq!(read_frame(&mut Cursor::new(bytes)).unwrap(), request);
        }
    }

    #[test]
    fn truncated_and_oversized_frames_are_rejected() {
        let bytes = encode_frame(&IpcRequest::NewWindow(Some(absolute()))).unwrap();
        assert!(read_frame(&mut Cursor::new(&bytes[..3])).is_err());
        assert!(read_frame(&mut Cursor::new(&bytes[..bytes.len() - 1])).is_err());
        assert!(decode_frame(&bytes[LENGTH_PREFIX..LENGTH_PREFIX + PAYLOAD_HEADER - 1]).is_err());
        let mut huge = ((MAX_PAYLOAD + 1) as u32).to_le_bytes().to_vec();
        huge.resize(MAX_PAYLOAD + LENGTH_PREFIX + 1, 0);
        assert!(read_frame(&mut Cursor::new(huge)).is_err());
    }

    #[test]
    fn invalid_utf8_and_paths_are_rejected() {
        let mut payload = Vec::new();
        payload.extend_from_slice(MAGIC);
        payload.extend_from_slice(&[VERSION, KIND_NEW_WINDOW, 1, 0, 0xff]);
        assert!(decode_frame(&payload).unwrap_err().contains("UTF-8"));
        assert!(encode_frame(&IpcRequest::NewWindow(Some(PathBuf::from("relative.md")))).is_err());
        assert!(encode_frame(&IpcRequest::NewWindow(Some(PathBuf::new()))).is_err());
    }

    #[test]
    fn malformed_flags_and_boundaries_are_rejected() {
        let bytes = encode_frame(&IpcRequest::NewWindow(None)).unwrap();
        let mut payload = bytes[4..].to_vec();
        payload[6] = 2;
        assert!(decode_frame(&payload).is_err());
        let mut payload = bytes[4..].to_vec();
        payload.push(b'x');
        assert!(decode_frame(&payload).is_err());
        let mut payload = bytes[4..].to_vec();
        payload[4] = VERSION + 1;
        assert!(decode_frame(&payload).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn named_pipe_loopback_opens_a_new_window_request() {
        let endpoint = format!(
            r"\\.\pipe\Veil.Test.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let listener = super::imp::listen_on(endpoint.clone()).unwrap();
        let expected = IpcRequest::NewWindow(Some(absolute()));
        let sent = expected.clone();
        let client = std::thread::spawn(move || super::imp::send_to(endpoint, &sent));
        let actual = listener
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        client.join().unwrap().unwrap();
        assert_eq!(actual, expected);
    }

    /// DEF-T10-004：慢客户端（连上不发帧）必须在读超时后被断开，监听恢复可用；
    /// 隔离本进程命名管道，不触碰真实 Veil 实例。
    #[cfg(windows)]
    #[test]
    fn slow_client_times_out_and_listener_recovers() {
        use std::fs::OpenOptions;
        let endpoint = format!(
            r"\\.\pipe\Veil.Test.Slow.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let listener = super::imp::listen_on(endpoint.clone()).unwrap();
        // 连上但不发送任何字节。
        let mut slow = None;
        for _ in 0..100 {
            match OpenOptions::new().write(true).open(&endpoint) {
                Ok(file) => {
                    slow = Some(file);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }
        let slow = slow.expect("应能连上隔离命名管道");
        let expected = IpcRequest::NewWindow(Some(absolute()));
        let sent = expected.clone();
        let client = std::thread::spawn(move || super::imp::send_to(endpoint, &sent));
        let actual = listener
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("慢客户端超时后监听应恢复并收到第二实例的帧");
        client.join().unwrap().unwrap();
        drop(slow);
        assert_eq!(actual, expected);
    }
}
