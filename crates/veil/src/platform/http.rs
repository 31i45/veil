//! 远程图片抓取（v1.4）：WinHTTP 手写 FFI（零新增依赖）+ 后台线程 + WIC 解码。
//!
//! 安全边界（与 shell 打开链接同一立场）：只允许 http/https 公网目标，拒绝用户信息、
//! 本机/私网地址与数字型主机（防 `2130706433`、`0x7f000001` 绕过）；会话禁用 Cookie、
//! 认证与自动重定向，跳转由本模块逐跳复核（最多 3 次，https→http 直接拒绝），
//! 不发送 Referer。响应体上限 20 MiB。
//!
//! 线程模型：`HttpFetcher` 留在 UI 线程（不跨线程共享），`request` 只投递任务并立刻
//! 返回，结果经 mpsc 回传，`poll` 非阻塞取走；网络/解码失败都变成可读 Err（视图降级
//! alt 文本），任何失败路径都不 panic。

#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, Sender};

use veil_ui::ImageData;

/// URL 长度上限（trim 后，字节）。
const MAX_URL_LEN: usize = 2048;
/// 响应体上限：20 MiB（远大于常见截图，超出视为异常输入）。
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// 手动跟跳次数上限。
const MAX_REDIRECTS: usize = 3;
/// 解析/连接/发送/接收统一超时（毫秒）。
const TIMEOUT_MS: i32 = 10_000;

/// 响应体是否超限：严格大于上限才拒绝（恰好 20 MiB 视为可接受）。
fn exceeds_body_limit(len: u64) -> bool {
    len > MAX_BYTES
}

/// 已校验的远程图片地址：方案 + 主机 + 端口 + 请求目标（路径 + 查询串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HttpUrl {
    /// https（其余为 http）。
    pub(crate) secure: bool,
    /// 主机名/IP（IPv6 已去掉方括号，供 WinHTTP 直接使用）。
    pub(crate) host: String,
    pub(crate) port: u16,
    /// 路径 + 查询串，以 `/` 开头；fragment 已丢弃。
    pub(crate) target: String,
}

impl HttpUrl {
    fn scheme(&self) -> &'static str {
        if self.secure {
            "https"
        } else {
            "http"
        }
    }

    /// `host[:port]`（IPv6 补回方括号；默认端口省略）。
    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let default_port = if self.secure { 443 } else { 80 };
        if self.port == default_port {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    fn origin(&self) -> String {
        format!("{}://{}", self.scheme(), self.authority())
    }

    /// 不含查询串的请求路径。
    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("/")
    }
}

/// 校验并解析远程图片 URL（纯函数，可单测）。拒绝本机/私网/数字型主机与用户信息。
pub(crate) fn validate_url(url: &str) -> Result<HttpUrl, String> {
    let s = url.trim();
    if s.is_empty() {
        return Err("URL 为空".into());
    }
    if s.len() > MAX_URL_LEN {
        return Err(format!("URL 过长（上限 {MAX_URL_LEN} 字符）"));
    }
    if s.chars().any(char::is_control) {
        return Err("URL 含控制字符".into());
    }
    let (scheme, rest) = s
        .split_once("://")
        .ok_or_else(|| "URL 缺少协议头（仅支持 http/https）".to_string())?;
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "http" => false,
        "https" => true,
        other => return Err(format!("不支持的协议：{other}")),
    };
    // authority 到首个 / ? # 为止
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() {
        return Err("URL 缺少主机名".into());
    }
    if authority.contains('@') {
        return Err("URL 不允许包含用户信息".into());
    }
    let (host, port) = split_authority(authority, secure)?;
    validate_host(&host, port_was_bracketed(authority))?;
    let target = request_target(&rest[end..]);
    Ok(HttpUrl {
        secure,
        host,
        port,
        target,
    })
}

/// 是否带 IPv6 方括号（`[::1]:80` 形式）。
fn port_was_bracketed(authority: &str) -> bool {
    authority.starts_with('[')
}

/// 拆出 host 与 port（缺省端口按方案取 80/443）。
fn split_authority(authority: &str, secure: bool) -> Result<(String, u16), String> {
    let (host, port_raw) = if let Some(rest) = authority.strip_prefix('[') {
        let close = rest
            .find(']')
            .ok_or_else(|| "IPv6 主机缺少右括号".to_string())?;
        let host = &rest[..close];
        let tail = &rest[close + 1..];
        let port = match tail {
            "" => None,
            t => Some(
                t.strip_prefix(':')
                    .ok_or_else(|| "主机名后存在非法字符".to_string())?,
            ),
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => {
                if h.contains(':') {
                    return Err("IPv6 主机必须用方括号包裹".into());
                }
                (h.to_string(), Some(p))
            }
            None => (authority.to_string(), None),
        }
    };
    let port = match port_raw {
        None => {
            if secure {
                443
            } else {
                80
            }
        }
        Some(p) => {
            if p.is_empty() {
                return Err("端口为空".into());
            }
            if !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("端口非法：{p}"));
            }
            let n: u32 = p.parse().map_err(|_| format!("端口非法：{p}"))?;
            if n == 0 || n > 65535 {
                return Err(format!("端口超出范围：{p}"));
            }
            n as u16
        }
    };
    Ok((host, port))
}

/// 主机合法性：拒绝本机/私网/保留地址与数字/十六进制字面量主机。
fn validate_host(host: &str, bracketed: bool) -> Result<(), String> {
    if host.is_empty() {
        return Err("URL 缺少主机名".into());
    }
    if host
        .chars()
        .any(|c| c == '\\' || c.is_whitespace() || c.is_control())
    {
        return Err("主机名含非法字符".into());
    }
    let ip: Option<std::net::IpAddr> = host.parse().ok();
    if bracketed && ip.is_none() {
        return Err(format!("IPv6 地址非法：{host}"));
    }
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err("不允许访问本机地址".into());
    }
    match ip {
        Some(std::net::IpAddr::V4(v4)) => {
            if blocked_v4(v4) {
                return Err(format!("不允许访问内网/保留地址：{host}"));
            }
        }
        Some(std::net::IpAddr::V6(v6)) => {
            if blocked_v6(v6) {
                return Err(format!("不允许访问内网/保留地址：{host}"));
            }
        }
        None => {
            // 浏览器会把 2130706433 / 0x7f000001 当 IPv4 解析：数字/十六进制主机一律拒绝
            if numeric_like_host(&lower) {
                return Err(format!("不允许数字/十六进制主机名：{host}"));
            }
            if !valid_domain(&lower) {
                return Err(format!("主机名非法：{host}"));
            }
        }
    }
    Ok(())
}

fn blocked_v4(addr: std::net::Ipv4Addr) -> bool {
    let o = addr.octets();
    addr.is_loopback()
        || addr.is_private()
        || addr.is_link_local()
        || addr.is_unspecified()
        || addr.is_broadcast()
        || o[0] == 0 // 0.0.0.0/8
        || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64.0.0/10
}

fn blocked_v6(addr: std::net::Ipv6Addr) -> bool {
    let o = addr.octets();
    if addr.is_loopback() || addr.is_unspecified() {
        return true;
    }
    if o[0] & 0xfe == 0xfc {
        return true; // fc00::/7
    }
    if o[0] == 0xfe && o[1] & 0xc0 == 0x80 {
        return true; // fe80::/10
    }
    // IPv4-mapped（::ffff:a.b.c.d）/ IPv4-compatible：按内嵌 IPv4 复核
    match addr.to_ipv4() {
        Some(v4) => blocked_v4(v4),
        None => false,
    }
}

/// 数字/十六进制字面量主机（无字母，或 `0x` + 全十六进制）。
fn numeric_like_host(host: &str) -> bool {
    let h = host.strip_suffix('.').unwrap_or(host);
    if !h.bytes().any(|b| b.is_ascii_alphabetic()) {
        return true;
    }
    if let Some(rest) = h.strip_prefix("0x").or_else(|| h.strip_prefix("0X")) {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_hexdigit());
    }
    false
}

/// 域名形态：标签 1..=63，字母数字/连字符，不以连字符开头或结尾。
fn valid_domain(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

/// 请求目标：路径（缺省 `/`）+ `?query`，丢弃 fragment。
fn request_target(rest: &str) -> String {
    let rest = match rest.find('#') {
        Some(i) => &rest[..i],
        None => rest,
    };
    if rest.is_empty() {
        return "/".to_string();
    }
    if rest.starts_with('/') {
        rest.to_string()
    } else {
        // 以 ? 开头（authority 后无路径）：补上根路径
        format!("/{rest}")
    }
}

/// 解析重定向地址（纯函数，可单测）：绝对 / 协议相对 / 路径绝对 / 路径相对。
pub(crate) fn resolve_redirect(base: &HttpUrl, location: &str) -> Result<HttpUrl, String> {
    let loc = location.trim();
    if loc.is_empty() {
        return Err("重定向地址为空".into());
    }
    if loc.contains("://") {
        return validate_url(loc);
    }
    if let Some(rest) = loc.strip_prefix("//") {
        return validate_url(&format!("{}://{}", base.scheme(), rest));
    }
    if let Some(rest) = loc.strip_prefix('?') {
        // query-only：保留当前路径，只替换查询串（DEF-T10-007）。
        return validate_url(&format!("{}{}?{rest}", base.origin(), base.path()));
    }
    if loc.starts_with('#') {
        // fragment-only：路径与查询串都沿用当前 URL，仅丢弃 fragment。
        return validate_url(&format!("{}{}", base.origin(), base.target));
    }
    if loc.starts_with('/') {
        return validate_url(&format!("{}{}", base.origin(), loc));
    }
    // 相对路径：按当前 URL 的目录解析（保留查询串的部分原样交给 validate_url）
    let path = base.path();
    let dir = &path[..path.rfind('/').map(|i| i + 1).unwrap_or(1)];
    let joined = normalize_path(&format!("{dir}{loc}"));
    validate_url(&format!("{}{}", base.origin(), joined))
}

/// 归并 `.` / `..` 段并保证以 `/` 开头。
fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    let mut out = String::from("/");
    out.push_str(&segments.join("/"));
    if path.ends_with('/') && out.len() > 1 {
        out.push('/');
    }
    out
}

/// `https → http` 降级（携带内容可能被窃听/篡改，直接拒绝）。
pub(crate) fn is_downgrade(from: &HttpUrl, to: &HttpUrl) -> bool {
    from.secure && !to.secure
}

/// 远程图片抓取器（UI 线程持有）：投递后台任务 + 非阻塞收集结果。
#[cfg(windows)]
pub struct HttpFetcher {
    tx: Sender<(String, Result<ImageData, String>)>,
    rx: Receiver<(String, Result<ImageData, String>)>,
}

#[cfg(windows)]
impl HttpFetcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }

    /// 校验 URL 后投递后台抓取；非法 URL 立即以错误结果回传（不发起网络请求）。
    pub fn request(&mut self, url: &str) {
        let url = url.trim().to_string();
        if let Err(e) = validate_url(&url) {
            let _ = self.tx.send((url, Err(e)));
            return;
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let label = url.clone();
            let result = fetch(&url);
            let _ = tx.send((label, result));
        });
    }

    /// 非阻塞取走全部已完成结果。
    pub fn poll(&mut self) -> Vec<(String, Result<ImageData, String>)> {
        let mut out = Vec::new();
        while let Ok(item) = self.rx.try_recv() {
            out.push(item);
        }
        out
    }
}

/// 非 Windows 兜底：保持 workspace 可编译，不发起任何请求。
#[cfg(not(windows))]
pub struct HttpFetcher;

#[cfg(not(windows))]
impl HttpFetcher {
    pub fn new() -> Self {
        Self
    }

    pub fn request(&mut self, _url: &str) {}

    pub fn poll(&mut self) -> Vec<(String, Result<ImageData, String>)> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Windows：WinHTTP 抓取
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    pub type Hinter = *mut c_void;

    #[link(name = "winhttp")]
    extern "system" {
        pub fn WinHttpOpen(
            agent: *const u16,
            access_type: u32,
            proxy: *const u16,
            bypass: *const u16,
            flags: u32,
        ) -> Hinter;
        pub fn WinHttpSetOption(
            handle: Hinter,
            option: u32,
            buffer: *mut c_void,
            length: u32,
        ) -> i32;
        pub fn WinHttpSetTimeouts(
            handle: Hinter,
            resolve: i32,
            connect: i32,
            send: i32,
            receive: i32,
        ) -> i32;
        pub fn WinHttpConnect(
            session: Hinter,
            server: *const u16,
            port: u16,
            reserved: u32,
        ) -> Hinter;
        pub fn WinHttpOpenRequest(
            connect: Hinter,
            verb: *const u16,
            object: *const u16,
            version: *const u16,
            referrer: *const u16,
            accept_types: *const *const u16,
            flags: u32,
        ) -> Hinter;
        pub fn WinHttpSendRequest(
            request: Hinter,
            headers: *const u16,
            headers_len: u32,
            optional: *mut c_void,
            optional_len: u32,
            total_len: u32,
            context: usize,
        ) -> i32;
        pub fn WinHttpReceiveResponse(request: Hinter, reserved: *mut c_void) -> i32;
        pub fn WinHttpQueryHeaders(
            request: Hinter,
            info_level: u32,
            name: *const u16,
            buffer: *mut c_void,
            buffer_len: *mut u32,
            index: *mut u32,
        ) -> i32;
        pub fn WinHttpReadData(
            request: Hinter,
            buffer: *mut c_void,
            to_read: u32,
            read: *mut u32,
        ) -> i32;
        pub fn WinHttpCloseHandle(handle: Hinter) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn GetLastError() -> u32;
    }
}

/// WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY：沿用系统代理配置（Win8.1+）。
#[cfg(windows)]
const WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY: u32 = 4;
/// WINHTTP_OPTION_DISABLE_FEATURE。
#[cfg(windows)]
const WINHTTP_OPTION_DISABLE_FEATURE: u32 = 109;
#[cfg(windows)]
const WINHTTP_DISABLE_COOKIES: u32 = 1;
#[cfg(windows)]
const WINHTTP_DISABLE_AUTHENTICATION: u32 = 2;
#[cfg(windows)]
const WINHTTP_DISABLE_REDIRECTS: u32 = 4;
/// WINHTTP_FLAG_SECURE：https 请求必须置位。
#[cfg(windows)]
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
/// WINHTTP_QUERY_STATUS_CODE / WINHTTP_QUERY_CONTENT_LENGTH / WINHTTP_QUERY_LOCATION。
#[cfg(windows)]
const WINHTTP_QUERY_CONTENT_LENGTH: u32 = 5;
#[cfg(windows)]
const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
#[cfg(windows)]
const WINHTTP_QUERY_LOCATION: u32 = 33;
/// WINHTTP_QUERY_FLAG_NUMBER：让 QueryHeaders 把值写入 DWORD 而不是字符串。
#[cfg(windows)]
const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

/// 一次请求的结果：200 带响应体，重定向带 Location，其余只有状态码。
#[cfg(windows)]
struct Hop {
    status: u32,
    location: Option<String>,
    body: Option<Vec<u8>>,
}

/// WinHTTP 句柄的 RAII 包装：离开作用域即关闭（含全部失败路径）。
#[cfg(windows)]
struct Internet(sys::Hinter);

#[cfg(windows)]
impl Internet {
    fn adopt(raw: sys::Hinter, what: &str) -> Result<Self, String> {
        if raw.is_null() {
            Err(format!("{what}失败（错误码 {}）", last_error()))
        } else {
            Ok(Self(raw))
        }
    }

    fn raw(&self) -> sys::Hinter {
        self.0
    }
}

#[cfg(windows)]
impl Drop for Internet {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: 句柄由本模块创建且只在这里关闭一次。
            unsafe {
                sys::WinHttpCloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn last_error() -> u32 {
    // SAFETY: GetLastError 无前置条件。
    unsafe { sys::GetLastError() }
}

/// 抓取并按 WIC 解码远程图片：只接受 200，手动跟跳（最多 3 次），响应体上限 20 MiB。
#[cfg(windows)]
fn fetch(url: &str) -> Result<ImageData, String> {
    let mut current = validate_url(url)?;
    let mut hops = 0usize;
    loop {
        let hop = fetch_once(&current)?;
        match hop.status {
            200 => {
                let body = hop.body.unwrap_or_default();
                return decode_bytes(&body);
            }
            301 | 302 | 303 | 307 | 308 => {
                if hops >= MAX_REDIRECTS {
                    return Err(format!("重定向次数超过 {MAX_REDIRECTS} 次"));
                }
                let location = hop
                    .location
                    .ok_or_else(|| "重定向响应缺少 Location".to_string())?;
                let next = resolve_redirect(&current, &location)?;
                if is_downgrade(&current, &next) {
                    return Err("拒绝从 https 跳转到 http".to_string());
                }
                current = next;
                hops += 1;
            }
            status => return Err(format!("HTTP {status}")),
        }
    }
}

#[cfg(windows)]
fn fetch_once(url: &HttpUrl) -> Result<Hop, String> {
    let session = open_session()?;
    let connect = open_connect(&session, url)?;
    let request = open_request(&connect, url)?;

    let ok = unsafe {
        sys::WinHttpSendRequest(
            request.raw(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    if ok == 0 {
        return Err(format!("发送请求失败（错误码 {}）", last_error()));
    }
    let ok = unsafe { sys::WinHttpReceiveResponse(request.raw(), std::ptr::null_mut()) };
    if ok == 0 {
        return Err(format!("接收响应失败（错误码 {}）", last_error()));
    }

    let status = query_status(&request)?;
    if status == 200 {
        check_content_length(&request)?;
        let body = read_body(&request)?;
        return Ok(Hop {
            status,
            location: None,
            body: Some(body),
        });
    }
    let location = if matches!(status, 301 | 302 | 303 | 307 | 308) {
        query_header_string(&request, WINHTTP_QUERY_LOCATION)?
    } else {
        None
    };
    Ok(Hop {
        status,
        location,
        body: None,
    })
}

#[cfg(windows)]
fn open_session() -> Result<Internet, String> {
    let agent = super::wide(&format!("Veil/{}", env!("CARGO_PKG_VERSION")));
    let raw = unsafe {
        sys::WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    let session = Internet::adopt(raw, "打开 WinHTTP 会话")?;
    let ok = unsafe {
        sys::WinHttpSetTimeouts(
            session.raw(),
            TIMEOUT_MS,
            TIMEOUT_MS,
            TIMEOUT_MS,
            TIMEOUT_MS,
        )
    };
    if ok == 0 {
        return Err(format!("设置 WinHTTP 超时失败（错误码 {}）", last_error()));
    }
    Ok(session)
}

#[cfg(windows)]
fn open_connect(session: &Internet, url: &HttpUrl) -> Result<Internet, String> {
    let host = super::wide(&url.host);
    let raw = unsafe { sys::WinHttpConnect(session.raw(), host.as_ptr(), url.port, 0) };
    Internet::adopt(raw, "连接远程主机")
}

#[cfg(windows)]
fn open_request(connect: &Internet, url: &HttpUrl) -> Result<Internet, String> {
    let verb = super::wide("GET");
    let target = super::wide(&url.target);
    let flags = if url.secure { WINHTTP_FLAG_SECURE } else { 0 };
    let raw = unsafe {
        sys::WinHttpOpenRequest(
            connect.raw(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            flags,
        )
    };
    let request = Internet::adopt(raw, "创建 HTTP 请求")?;
    // 禁 Cookie/认证/自动重定向：抓图不携带会话凭据，跳转必须逐跳复核。
    // WINHTTP_OPTION_DISABLE_FEATURE 只接受请求句柄；设在会话句柄会返回
    // ERROR_WINHTTP_ACCESS_TYPE_DENIED(12018)，导致请求根本发不出去。
    let disable =
        WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION | WINHTTP_DISABLE_REDIRECTS;
    let ok = unsafe {
        sys::WinHttpSetOption(
            request.raw(),
            WINHTTP_OPTION_DISABLE_FEATURE,
            &disable as *const u32 as *mut std::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    if ok == 0 {
        return Err(format!("配置 HTTP 请求失败（错误码 {}）", last_error()));
    }
    Ok(request)
}

#[cfg(windows)]
fn query_status(request: &Internet) -> Result<u32, String> {
    let mut value: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        sys::WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut value as *mut u32 as *mut std::ffi::c_void,
            &mut len,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!("读取响应状态失败（错误码 {}）", last_error()));
    }
    Ok(value)
}

/// 读字符串响应头；缺失/读取失败返回 None（由调用方给出可读错误）。
#[cfg(windows)]
fn query_header_string(request: &Internet, query: u32) -> Result<Option<String>, String> {
    let mut len = 0u32;
    unsafe {
        sys::WinHttpQueryHeaders(
            request.raw(),
            query,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
        )
    };
    if len == 0 {
        return Ok(None);
    }
    let mut buf = vec![0u16; len as usize + 1];
    let mut cap = buf.len() as u32;
    let ok = unsafe {
        sys::WinHttpQueryHeaders(
            request.raw(),
            query,
            std::ptr::null(),
            buf.as_mut_ptr() as *mut std::ffi::c_void,
            &mut cap,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Ok(None);
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(Some(String::from_utf16_lossy(&buf[..n])))
}

/// Content-Length 已知且超限时先拒绝，避免下载大文件。
#[cfg(windows)]
fn check_content_length(request: &Internet) -> Result<(), String> {
    let mut value: u32 = 0;
    let mut len = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        sys::WinHttpQueryHeaders(
            request.raw(),
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut value as *mut u32 as *mut std::ffi::c_void,
            &mut len,
            std::ptr::null_mut(),
        )
    };
    if ok != 0 && exceeds_body_limit(u64::from(value)) {
        return Err(too_large_text(u64::from(value)));
    }
    Ok(())
}

#[cfg(windows)]
fn read_body(request: &Internet) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        let ok = unsafe {
            sys::WinHttpReadData(
                request.raw(),
                chunk.as_mut_ptr() as *mut std::ffi::c_void,
                chunk.len() as u32,
                &mut read,
            )
        };
        if ok == 0 {
            return Err(format!("读取响应数据失败（错误码 {}）", last_error()));
        }
        if read == 0 {
            break;
        }
        out.extend_from_slice(&chunk[..read as usize]);
        if exceeds_body_limit(out.len() as u64) {
            return Err(too_large_text(out.len() as u64));
        }
    }
    Ok(out)
}

#[cfg(windows)]
fn too_large_text(bytes: u64) -> String {
    format!(
        "远程图片超过 {} MiB 上限（{} 字节）",
        MAX_BYTES / (1024 * 1024),
        bytes
    )
}

/// 响应字节 → 临时文件 → WIC 解码（完成后删除；失败只返回可读错误）。
#[cfg(windows)]
fn decode_bytes(bytes: &[u8]) -> Result<ImageData, String> {
    if bytes.is_empty() {
        return Err("远程图片响应为空".into());
    }
    let path = temp_path(bytes);
    if let Err(e) = std::fs::write(&path, bytes) {
        return Err(format!("写入远程图片临时文件失败：{e}"));
    }
    let decoded = super::decode_image(&path);
    let _ = std::fs::remove_file(&path);
    decoded
}

/// 唯一临时文件名：pid + 原子计数 + 纳秒；扩展名按内容嗅探（WIC 按扩展名选解码器）。
#[cfg(windows)]
fn temp_path(bytes: &[u8]) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "veil-remote-{}-{}-{}{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed),
        nanos,
        image_ext(bytes)
    ))
}

#[cfg(windows)]
fn image_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        ".png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ".jpg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        ".gif"
    } else if bytes.starts_with(b"BM") {
        ".bmp"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ".webp"
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        ".tif"
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        ".ico"
    } else {
        ".png"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_public_http_and_https() {
        let http = validate_url("http://example.com/a.png").unwrap();
        assert!(!http.secure);
        assert_eq!(http.host, "example.com");
        assert_eq!(http.port, 80);
        assert_eq!(http.target, "/a.png");
        assert_eq!(http.origin(), "http://example.com");

        let https = validate_url("https://example.com").unwrap();
        assert!(https.secure);
        assert_eq!(https.port, 443);
        assert_eq!(https.target, "/");
        assert_eq!(https.origin(), "https://example.com");

        // 大小写不敏感 + 端口 + 查询串保留 + fragment 丢弃
        let custom = validate_url("  HTTPS://Example.COM:8443/x/y?w=1#frag  ").unwrap();
        assert!(custom.secure);
        assert_eq!(custom.host, "Example.COM");
        assert_eq!(custom.port, 8443);
        assert_eq!(custom.target, "/x/y?w=1");
        assert_eq!(custom.origin(), "https://Example.COM:8443");

        // 公网 IP 允许
        assert!(validate_url("http://8.8.8.8/a.jpg").is_ok());
        assert!(validate_url("https://[2001:4860:4860::8888]/x").is_ok());
    }

    #[test]
    fn rejects_unsafe_or_malformed_urls() {
        for bad in [
            "",
            "   ",
            "ftp://example.com/a.png",
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "http://",
            "http:///a.png",
            "http://user@example.com/a.png",
            "http://user:pass@example.com/a.png",
            "http://localhost/a.png",
            "http://localhost:8080/a.png",
            "http://a.localhost/a.png",
            "http://127.0.0.1/a.png",
            "http://10.1.2.3/a.png",
            "http://172.16.0.1/a.png",
            "http://192.168.1.1/a.png",
            "http://169.254.1.1/a.png",
            "http://[::1]/a.png",
            "http://[fe80::1]/a.png",
            "http://[fd00::1]/a.png",
            "http://[::ffff:127.0.0.1]/a.png",
            "http://0.0.0.0/a.png",
            "http://255.255.255.255/a.png",
            "http://100.64.0.1/a.png",
            "http://2130706433/",
            "http://0x7f000001/",
            "http://127.1/",
            "http://exa mple.com/a.png",
            "http://example.com:0/a.png",
            "http://example.com:70000/a.png",
            "http://example.com:/a.png",
            "http://[::1/a.png",
            "http://::1/a.png",
        ] {
            assert!(validate_url(bad).is_err(), "应拒绝：{bad}");
        }
        let long = format!("http://example.com/{}", "a".repeat(2100));
        assert!(validate_url(&long).is_err(), "超长 URL 应拒绝");
    }

    #[test]
    fn limits_match_spec() {
        assert_eq!(MAX_BYTES, 20 * 1024 * 1024);
        assert_eq!(MAX_REDIRECTS, 3);
        assert_eq!(TIMEOUT_MS, 10_000);
    }

    /// 响应体上限边界：恰好 20 MiB 可接受，多 1 字节即拒绝。
    #[test]
    fn body_limit_boundary_rejects_only_above_cap() {
        assert!(!exceeds_body_limit(0));
        assert!(!exceeds_body_limit(MAX_BYTES - 1));
        assert!(!exceeds_body_limit(MAX_BYTES), "恰好 20 MiB 应可接受");
        assert!(exceeds_body_limit(MAX_BYTES + 1), "20 MiB + 1 字节应拒绝");
    }

    #[test]
    fn resolves_redirect_forms() {
        let base = validate_url("https://example.com/a/b.png?w=1").unwrap();

        let absolute = resolve_redirect(&base, "https://cdn.example.org/x.jpg").unwrap();
        assert_eq!(absolute.host, "cdn.example.org");
        assert_eq!(absolute.target, "/x.jpg");
        assert!(absolute.secure);

        let protocol_relative = resolve_redirect(&base, "//cdn.example.org/y.png").unwrap();
        assert_eq!(protocol_relative.origin(), "https://cdn.example.org");
        assert_eq!(protocol_relative.target, "/y.png");

        let path_absolute = resolve_redirect(&base, "/root.png").unwrap();
        assert_eq!(path_absolute.origin(), "https://example.com");
        assert_eq!(path_absolute.target, "/root.png");

        let path_relative = resolve_redirect(&base, "c.png").unwrap();
        assert_eq!(path_relative.target, "/a/c.png");

        let parent = resolve_redirect(&base, "../d.png").unwrap();
        assert_eq!(parent.target, "/d.png");

        let deeper = resolve_redirect(&base, "img/e.png").unwrap();
        assert_eq!(deeper.target, "/a/img/e.png");

        // query-only / fragment-only：相对当前 URL 解析，不得丢掉当前路径
        // （DEF-T10-007）。
        let query_only = resolve_redirect(&base, "?download=1").unwrap();
        assert_eq!(query_only.origin(), "https://example.com");
        assert_eq!(query_only.target, "/a/b.png?download=1");
        let fragment_only = resolve_redirect(&base, "#section").unwrap();
        assert_eq!(fragment_only.origin(), "https://example.com");
        assert_eq!(
            fragment_only.target, "/a/b.png?w=1",
            "fragment-only 保留路径与查询串，仅丢弃 fragment"
        );
    }

    #[test]
    fn redirect_downgrade_is_rejected() {
        let https = validate_url("https://example.com/a.png").unwrap();
        let http = validate_url("http://example.com/b.png").unwrap();
        let next_https = validate_url("https://cdn.example.com/c.png").unwrap();

        assert!(is_downgrade(&https, &http), "https→http 必须判定为降级");
        assert!(!is_downgrade(&http, &https));
        assert!(!is_downgrade(&https, &next_https));

        // 每跳都走同一校验：降级目标或内网目标在 resolve 阶段即被拒绝
        assert!(resolve_redirect(&https, "http://127.0.0.1/x.png").is_err());
        assert!(resolve_redirect(&https, "http://example.com/x.png").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn invalid_request_returns_readable_error_without_network() {
        let mut fetcher = HttpFetcher::new();
        assert!(fetcher.poll().is_empty(), "初始无结果");
        fetcher.request("ftp://example.com/a.png");
        let got = fetcher.poll();
        assert_eq!(got.len(), 1, "非法 URL 立即回传错误");
        assert_eq!(got[0].0, "ftp://example.com/a.png");
        assert!(got[0].1.is_err());
        assert!(fetcher.poll().is_empty(), "结果只取走一次");
    }
}
