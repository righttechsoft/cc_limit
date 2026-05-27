#![windows_subsystem = "windows"]

use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use serde_json::{json, Value};

use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, LRESULT, TRUE, UINT, WPARAM};
use winapi::shared::windef::{HBRUSH, HMENU, HMONITOR, HWND, RECT};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::shellapi::{
    SHAppBarMessage, Shell_NotifyIconW, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
    APPBARDATA, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use winapi::um::wingdi::{
    CreateSolidBrush, DeleteObject, GetStockObject, SelectObject, SetBkMode, SetTextColor,
    TextOutW, DEFAULT_GUI_FONT, TRANSPARENT,
};
use winapi::um::winhttp::{
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE,
};
use winapi::um::winuser::{
    AppendMenuW, BeginPaint, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, EndPaint, EnumDisplayMonitors, FillRect, GetClientRect, GetCursorPos,
    GetMessageW, GetMonitorInfoW, InvalidateRect, KillTimer, LoadCursorW, LoadIconW, MoveWindow,
    PostQuitMessage, RegisterClassW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos,
    ShowWindow, TrackPopupMenu, TranslateMessage, COLOR_WINDOW, CS_HREDRAW, CS_VREDRAW,
    HWND_TOPMOST, IDC_ARROW, IDI_APPLICATION, MAKEINTRESOURCEW, MF_CHECKED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    MONITORINFOEXW, MONITORINFOF_PRIMARY, MSG, PAINTSTRUCT, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SW_SHOW, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_DISPLAYCHANGE, WM_LBUTTONUP, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WM_USER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

const WINHTTP_ADDREQ_FLAG_ADD: DWORD = 0x20000000;

const TIMER_REFRESH: usize = 1;
const TIMER_RETRY: usize = 2;
const REFRESH_MS: UINT = 60_000;
const RETRY_MS: UINT = 15_000;
const BAR_HEIGHT: i32 = 18;
const TRAY_ID: u32 = 1;
const TRAY_CALLBACK: UINT = WM_USER + 1;
const APPBAR_CALLBACK: UINT = WM_USER + 2;

const ID_QUIT: u16 = 1001;
const ID_REFRESH: u16 = 1002;
const ID_SCREEN_BASE: u16 = 2000;

const COLOR_BG: u32 = 0x00F5F5F5;
const COLOR_TRACK: u32 = 0x00E2E2E2;
const COLOR_FILL: u32 = 0x00E07020;
const COLOR_FILL_WARN: u32 = 0x002080F0;
const COLOR_FILL_CRIT: u32 = 0x002020F0;
const COLOR_TEXT: u32 = 0x00404040;

#[derive(Clone)]
struct MonitorInfo {
    device: String,
    rect: RECT,
    primary: bool,
}

#[derive(Clone)]
struct AppState {
    used_pct: Option<f64>,
    resets_at_epoch: Option<i64>,
    label: String,
    error: Option<String>,
    monitors: Vec<MonitorInfo>,
    selected_device: Option<String>,
    appbar_registered: bool,
}

static STATE: Lazy<Mutex<AppState>> = Lazy::new(|| {
    Mutex::new(AppState {
        used_pct: None,
        resets_at_epoch: None,
        label: "loading…".into(),
        error: None,
        monitors: Vec::new(),
        selected_device: load_config_device(),
        appbar_registered: false,
    })
});

fn wstr(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn config_path() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    let dir = local.join("cc_limit");
    let _ = fs::create_dir_all(&dir);
    dir.join("config.json")
}

fn debug_path() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    let dir = local.join("cc_limit");
    let _ = fs::create_dir_all(&dir);
    dir.join("last_response.json")
}

fn load_config_device() -> Option<String> {
    let txt = fs::read_to_string(config_path()).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    v.get("monitor_device").and_then(|x| x.as_str()).map(String::from)
}

fn save_config_device(device: &str) {
    let v = json!({ "monitor_device": device });
    let _ = fs::write(config_path(), v.to_string());
}

fn credentials_path() -> PathBuf {
    let profile = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    profile.join(".claude").join(".credentials.json")
}

fn read_access_token() -> Result<String, String> {
    let path = credentials_path();
    let txt = fs::read_to_string(&path).map_err(|e| format!("cred read: {}", e))?;
    let v: Value = serde_json::from_str(&txt).map_err(|e| format!("cred parse: {}", e))?;
    if let Some(t) = find_string_key(&v, "accessToken") {
        return Ok(t);
    }
    if let Some(t) = find_string_key(&v, "access_token") {
        return Ok(t);
    }
    Err("no accessToken in credentials".into())
}

fn find_string_key(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(s)) = m.get(key) {
                return Some(s.clone());
            }
            for (_, child) in m {
                if let Some(s) = find_string_key(child, key) {
                    return Some(s);
                }
            }
            None
        }
        Value::Array(a) => {
            for child in a {
                if let Some(s) = find_string_key(child, key) {
                    return Some(s);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn extract_epoch(v: &Value) -> Option<i64> {
    if let Some(n) = v.as_i64() {
        if n > 10_000_000_000 {
            return Some(n / 1000);
        }
        return Some(n);
    }
    if let Some(s) = v.as_str() {
        return iso8601_to_epoch(s).ok();
    }
    None
}

fn iso8601_to_epoch(s: &str) -> Result<i64, ()> {
    let s = s.trim();
    let (date, mut tail) = s.split_once('T').ok_or(())?;
    let mut offset_secs: i64 = 0;
    if let Some(stripped) = tail.strip_suffix('Z') {
        tail = stripped;
    } else if let Some(idx) = tail.rfind(|c: char| c == '+' || c == '-') {
        if idx > 0 {
            let sign = if &tail[idx..idx + 1] == "+" { 1 } else { -1 };
            let off = &tail[idx + 1..];
            let mut op = off.split(':');
            let oh: i64 = op.next().ok_or(())?.parse().map_err(|_| ())?;
            let om: i64 = op.next().unwrap_or("0").parse().map_err(|_| ())?;
            offset_secs = sign * (oh * 3600 + om * 60);
            tail = &tail[..idx];
        }
    }
    let mut date_parts = date.split('-');
    let y: i32 = date_parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let mo: u32 = date_parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let d: u32 = date_parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let mut tp = tail.split(':');
    let h: u32 = tp.next().ok_or(())?.parse().map_err(|_| ())?;
    let mi: u32 = tp.next().ok_or(())?.parse().map_err(|_| ())?;
    let se_str = tp.next().unwrap_or("0");
    let se: u32 = se_str.split('.').next().unwrap_or("0").parse().map_err(|_| ())?;
    Ok(days_from_civil(y, mo, d) * 86400 + (h as i64) * 3600 + (mi as i64) * 60 + se as i64
        - offset_secs)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let d = d as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_usage(resp: &str) -> Result<(f64, Option<i64>), String> {
    let v: Value = serde_json::from_str(resp).map_err(|e| format!("json: {}", e))?;
    let five_hour = v.get("five_hour").ok_or_else(|| "no five_hour field".to_string())?;
    let pct = ["utilization", "percent", "used", "percentage"]
        .iter()
        .find_map(|k| five_hour.get(*k).and_then(extract_number))
        .or_else(|| extract_number(five_hour))
        .ok_or_else(|| "no pct in five_hour".to_string())?;
    let pct = if pct <= 1.0 { pct * 100.0 } else { pct };
    let reset = ["resets_at", "reset_at", "reset"]
        .iter()
        .find_map(|k| five_hour.get(*k).and_then(extract_epoch));
    Ok((pct, reset))
}

fn fetch_usage(token: &str) -> Result<String, String> {
    unsafe {
        let agent = wstr("cc_limit/0.1");
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if session.is_null() {
            return Err(format!("WinHttpOpen {}", GetLastError()));
        }
        let host = wstr("api.anthropic.com");
        let conn = WinHttpConnect(session, host.as_ptr(), 443, 0);
        if conn.is_null() {
            let e = GetLastError();
            WinHttpCloseHandle(session);
            return Err(format!("WinHttpConnect {}", e));
        }
        let verb = wstr("GET");
        let path = wstr("/api/oauth/usage");
        let req = WinHttpOpenRequest(
            conn,
            verb.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        );
        if req.is_null() {
            let e = GetLastError();
            WinHttpCloseHandle(conn);
            WinHttpCloseHandle(session);
            return Err(format!("WinHttpOpenRequest {}", e));
        }
        let headers = wstr(&format!(
            "Authorization: Bearer {}\r\nAccept: application/json\r\nanthropic-beta: oauth-2025-04-20",
            token
        ));
        let header_len = (headers.len() - 1) as DWORD;
        WinHttpAddRequestHeaders(req, headers.as_ptr(), header_len, WINHTTP_ADDREQ_FLAG_ADD);

        let ok = WinHttpSendRequest(req, ptr::null(), 0, ptr::null_mut(), 0, 0, 0);
        if ok == 0 {
            let e = GetLastError();
            WinHttpCloseHandle(req);
            WinHttpCloseHandle(conn);
            WinHttpCloseHandle(session);
            return Err(format!("WinHttpSendRequest {}", e));
        }
        let ok = WinHttpReceiveResponse(req, ptr::null_mut());
        if ok == 0 {
            let e = GetLastError();
            WinHttpCloseHandle(req);
            WinHttpCloseHandle(conn);
            WinHttpCloseHandle(session);
            return Err(format!("WinHttpReceiveResponse {}", e));
        }
        let mut out: Vec<u8> = Vec::new();
        loop {
            let mut avail: DWORD = 0;
            if WinHttpQueryDataAvailable(req, &mut avail) == 0 {
                break;
            }
            if avail == 0 {
                break;
            }
            let mut buf = vec![0u8; avail as usize];
            let mut read: DWORD = 0;
            if WinHttpReadData(req, buf.as_mut_ptr() as *mut _, avail, &mut read) == 0 {
                break;
            }
            buf.truncate(read as usize);
            out.extend_from_slice(&buf);
        }
        WinHttpCloseHandle(req);
        WinHttpCloseHandle(conn);
        WinHttpCloseHandle(session);
        String::from_utf8(out).map_err(|e| format!("utf8: {}", e))
    }
}

unsafe extern "system" fn monitor_enum_proc(
    h: HMONITOR,
    _hdc: winapi::shared::windef::HDC,
    _rc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = &mut *(lparam as *mut Vec<MonitorInfo>);
    let mut info: MONITORINFOEXW = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFOEXW>() as DWORD;
    if GetMonitorInfoW(h, &mut info as *mut _ as *mut _) == 0 {
        return 1;
    }
    let device_len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
    let device = String::from_utf16_lossy(&info.szDevice[..device_len]);
    list.push(MonitorInfo {
        device,
        rect: info.rcMonitor,
        primary: (info.dwFlags & MONITORINFOF_PRIMARY) != 0,
    });
    1
}

fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(monitor_enum_proc),
            &mut list as *mut _ as LPARAM,
        );
    }
    list
}

fn pick_monitor<'a>(monitors: &'a [MonitorInfo], selected: &Option<String>) -> Option<&'a MonitorInfo> {
    if let Some(d) = selected {
        if let Some(m) = monitors.iter().find(|m| &m.device == d) {
            return Some(m);
        }
    }
    monitors.iter().find(|m| m.primary).or_else(|| monitors.first())
}

unsafe fn position_window(hwnd: HWND) {
    let mut guard = STATE.lock().unwrap();
    guard.monitors = enumerate_monitors();
    let monitors = guard.monitors.clone();
    let selected = guard.selected_device.clone();
    let was_registered = guard.appbar_registered;
    drop(guard);

    let m = match pick_monitor(&monitors, &selected) {
        Some(m) => m.clone(),
        None => return,
    };

    if was_registered {
        let mut abd: APPBARDATA = std::mem::zeroed();
        abd.cbSize = std::mem::size_of::<APPBARDATA>() as DWORD;
        abd.hWnd = hwnd;
        SHAppBarMessage(ABM_REMOVE, &mut abd);
    }

    let mut abd: APPBARDATA = std::mem::zeroed();
    abd.cbSize = std::mem::size_of::<APPBARDATA>() as DWORD;
    abd.hWnd = hwnd;
    abd.uCallbackMessage = APPBAR_CALLBACK;
    if SHAppBarMessage(ABM_NEW, &mut abd) == 0 {
        let mut guard = STATE.lock().unwrap();
        guard.appbar_registered = false;
        drop(guard);
        MoveWindow(
            hwnd,
            m.rect.left,
            m.rect.top,
            m.rect.right - m.rect.left,
            BAR_HEIGHT,
            TRUE,
        );
        return;
    }

    abd.uEdge = ABE_TOP;
    abd.rc.left = m.rect.left;
    abd.rc.top = m.rect.top;
    abd.rc.right = m.rect.right;
    abd.rc.bottom = m.rect.top + BAR_HEIGHT;
    SHAppBarMessage(ABM_QUERYPOS, &mut abd);
    abd.rc.left = m.rect.left;
    abd.rc.right = m.rect.right;
    abd.rc.top = m.rect.top;
    abd.rc.bottom = m.rect.top + BAR_HEIGHT;
    SHAppBarMessage(ABM_SETPOS, &mut abd);

    MoveWindow(
        hwnd,
        abd.rc.left,
        abd.rc.top,
        abd.rc.right - abd.rc.left,
        abd.rc.bottom - abd.rc.top,
        TRUE,
    );
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
    );

    let mut guard = STATE.lock().unwrap();
    guard.appbar_registered = true;
}

unsafe fn remove_appbar(hwnd: HWND) {
    let guard = STATE.lock().unwrap();
    let was = guard.appbar_registered;
    drop(guard);
    if was {
        let mut abd: APPBARDATA = std::mem::zeroed();
        abd.cbSize = std::mem::size_of::<APPBARDATA>() as DWORD;
        abd.hWnd = hwnd;
        SHAppBarMessage(ABM_REMOVE, &mut abd);
        let mut g = STATE.lock().unwrap();
        g.appbar_registered = false;
    }
}

unsafe fn add_tray_icon(hwnd: HWND) {
    let hinst = GetModuleHandleW(ptr::null());
    let mut icon = LoadIconW(hinst, MAKEINTRESOURCEW(1));
    if icon.is_null() {
        icon = LoadIconW(ptr::null_mut(), IDI_APPLICATION);
    }
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as DWORD;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = TRAY_CALLBACK;
    nid.hIcon = icon;
    let tip = wstr("cc_limit");
    for (i, c) in tip.iter().enumerate().take(127) {
        nid.szTip[i] = *c;
    }
    Shell_NotifyIconW(NIM_ADD, &mut nid);
}

unsafe fn update_tray_tip(hwnd: HWND, label: &str) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as DWORD;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_TIP;
    let tip = wstr(label);
    for (i, c) in tip.iter().enumerate().take(127) {
        nid.szTip[i] = *c;
    }
    Shell_NotifyIconW(NIM_MODIFY, &mut nid);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as DWORD;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    Shell_NotifyIconW(NIM_DELETE, &mut nid);
}

fn refresh(hwnd_usize: usize) {
    let hwnd = hwnd_usize as HWND;
    let result: Result<(f64, Option<i64>), String> = (|| {
        let token = read_access_token()?;
        let body = fetch_usage(&token)?;
        let _ = fs::write(debug_path(), &body);
        parse_usage(&body)
    })();
    let tip;
    match result {
        Ok((pct, reset)) => {
            let label = format_label(pct, reset);
            {
                let mut guard = STATE.lock().unwrap();
                guard.used_pct = Some(pct);
                guard.resets_at_epoch = reset;
                guard.label = label.clone();
                guard.error = None;
            }
            tip = format!("cc_limit · {}", label);
            unsafe {
                KillTimer(hwnd, TIMER_RETRY);
            }
        }
        Err(e) => {
            let prev_label = {
                let mut guard = STATE.lock().unwrap();
                guard.error = Some(e.clone());
                guard.label.clone()
            };
            tip = format!("cc_limit · {} (transient err: {})", prev_label, e);
            unsafe {
                SetTimer(hwnd, TIMER_RETRY, RETRY_MS, None);
            }
        }
    }
    unsafe {
        InvalidateRect(hwnd, ptr::null(), TRUE);
        update_tray_tip(hwnd, &tip);
    }
}

fn format_label(pct: f64, reset: Option<i64>) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let suffix = match reset {
        Some(t) if t > now => {
            let mut s = t - now;
            let h = s / 3600;
            s %= 3600;
            let m = s / 60;
            format!(" · resets in {}h {}m", h, m)
        }
        _ => String::new(),
    };
    format!("{:.0}% used{}", pct, suffix)
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    let refresh_s = wstr("Refresh now");
    AppendMenuW(menu, MF_STRING, ID_REFRESH as usize, refresh_s.as_ptr());

    let screen_sub: HMENU = CreatePopupMenu();
    let guard = STATE.lock().unwrap();
    let monitors = guard.monitors.clone();
    let selected = guard.selected_device.clone();
    drop(guard);
    let chosen = pick_monitor(&monitors, &selected).map(|m| m.device.clone());
    for (i, m) in monitors.iter().enumerate() {
        let w = (m.rect.right - m.rect.left).abs();
        let h = (m.rect.bottom - m.rect.top).abs();
        let label = format!(
            "{} ({}×{}){}{}",
            i + 1,
            w,
            h,
            if m.primary { " · primary" } else { "" },
            if chosen.as_deref() == Some(m.device.as_str()) { " ✓" } else { "" }
        );
        let s = wstr(&label);
        let flags = MF_STRING
            | if chosen.as_deref() == Some(m.device.as_str()) {
                MF_CHECKED
            } else {
                0
            };
        AppendMenuW(
            screen_sub,
            flags,
            (ID_SCREEN_BASE as usize) + i,
            s.as_ptr(),
        );
    }
    let screen_label = wstr("Choose screen");
    AppendMenuW(
        menu,
        MF_POPUP | MF_STRING,
        screen_sub as usize,
        screen_label.as_ptr(),
    );

    AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    let quit_s = wstr("Quit");
    AppendMenuW(menu, MF_STRING, ID_QUIT as usize, quit_s.as_ptr());

    let mut pt = winapi::shared::windef::POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON | TPM_LEFTALIGN,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    );
    DestroyMenu(menu);
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            position_window(hwnd);
            add_tray_icon(hwnd);
            SetTimer(hwnd, TIMER_REFRESH, REFRESH_MS, None);
            let h = hwnd as usize;
            thread::spawn(move || refresh(h));
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let bg = CreateSolidBrush(COLOR_BG);
            FillRect(hdc, &rc, bg);
            DeleteObject(bg as _);

            let snapshot = STATE.lock().unwrap().clone();

            let track_left = 4;
            let track_right_pad = 240;
            let track_top = (rc.bottom - rc.top) / 2 - 4;
            let track_bot = track_top + 8;
            let track_rc = RECT {
                left: track_left,
                top: track_top,
                right: (rc.right - track_right_pad).max(track_left + 20),
                bottom: track_bot,
            };
            let track_brush = CreateSolidBrush(COLOR_TRACK);
            FillRect(hdc, &track_rc, track_brush);
            DeleteObject(track_brush as _);

            let pct = snapshot.used_pct.unwrap_or(0.0);
            let label = snapshot.label;
            let fill_color = if pct >= 90.0 {
                COLOR_FILL_CRIT
            } else if pct >= 70.0 {
                COLOR_FILL_WARN
            } else {
                COLOR_FILL
            };
            let width = (track_rc.right - track_rc.left) as f64;
            let fill_w = (width * (pct.clamp(0.0, 100.0) / 100.0)) as i32;
            if fill_w > 0 {
                let fill_rc = RECT {
                    left: track_rc.left,
                    top: track_rc.top,
                    right: track_rc.left + fill_w,
                    bottom: track_rc.bottom,
                };
                let fill_brush = CreateSolidBrush(fill_color);
                FillRect(hdc, &fill_rc, fill_brush);
                DeleteObject(fill_brush as _);
            }

            SetBkMode(hdc, TRANSPARENT as i32);
            SetTextColor(hdc, COLOR_TEXT);
            let font = GetStockObject(DEFAULT_GUI_FONT as i32);
            let old = SelectObject(hdc, font);
            let text = wstr(&label);
            let text_x = (rc.right - track_right_pad + 8).max(track_left);
            let text_y = (rc.bottom - rc.top) / 2 - 7;
            TextOutW(hdc, text_x, text_y, text.as_ptr(), (text.len() - 1) as i32);
            SelectObject(hdc, old);

            EndPaint(hwnd, &ps);
            0
        }
        WM_TIMER => {
            if wparam == TIMER_REFRESH || wparam == TIMER_RETRY {
                let h = hwnd as usize;
                thread::spawn(move || refresh(h));
            }
            0
        }
        m if m == TRAY_CALLBACK => {
            let event = (lparam & 0xFFFF) as UINT;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_tray_menu(hwnd);
            }
            0
        }
        m if m == APPBAR_CALLBACK => 0,
        WM_RBUTTONUP => {
            show_tray_menu(hwnd);
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as u16;
            match id {
                ID_QUIT => {
                    SendMessageW(hwnd, WM_DESTROY, 0, 0);
                }
                ID_REFRESH => {
                    let h = hwnd as usize;
                    thread::spawn(move || refresh(h));
                }
                x if x >= ID_SCREEN_BASE => {
                    let idx = (x - ID_SCREEN_BASE) as usize;
                    let mut g = STATE.lock().unwrap();
                    let monitors = g.monitors.clone();
                    if let Some(m) = monitors.get(idx) {
                        g.selected_device = Some(m.device.clone());
                        save_config_device(&m.device);
                    }
                    drop(g);
                    position_window(hwnd);
                    InvalidateRect(hwnd, ptr::null(), TRUE);
                }
                _ => {}
            }
            0
        }
        WM_DISPLAYCHANGE => {
            position_window(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_REFRESH);
            KillTimer(hwnd, TIMER_RETRY);
            remove_appbar(hwnd);
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        let hinst = GetModuleHandleW(ptr::null());
        let mut icon = LoadIconW(hinst, MAKEINTRESOURCEW(1));
        if icon.is_null() {
            icon = LoadIconW(ptr::null_mut(), IDI_APPLICATION);
        }
        let class_name = wstr("cc_limit_bar");
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.style = CS_HREDRAW | CS_VREDRAW;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.hIcon = icon;
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        wc.hbrBackground = (COLOR_WINDOW + 1) as HBRUSH;
        wc.lpszClassName = class_name.as_ptr();
        RegisterClassW(&wc);

        let title = wstr("cc_limit");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            800,
            BAR_HEIGHT,
            ptr::null_mut(),
            ptr::null_mut(),
            hinst,
            ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        ShowWindow(hwnd, SW_SHOW);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
