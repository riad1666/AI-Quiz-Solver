#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, COLOR_BTNFACE, DEFAULT_GUI_FONT, HBRUSH};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD, REG_SZ,
};
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::Shell::{
    IShellLinkW, SHBrowseForFolderW, SHGetFolderPathW, SHGetPathFromIDListW, ShellLink,
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, CSIDL_DESKTOPDIRECTORY,
    CSIDL_PROGRAMS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, LoadIconW,
    LoadImageW, MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage,
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, CREATESTRUCTW, CW_USEDEFAULT, ES_AUTOHSCROLL,
    GWLP_USERDATA, HICON, HMENU, IDI_APPLICATION, IDYES, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_YESNO, MSG, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, STM_SETICON, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_NCCREATE, WM_SETFONT, WNDCLASSW, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const SS_ICON: u32 = 0x0000_0003;

const APP_NAME: &str = "KNU Diagnosis";
const APP_PROVIDER: &str = "Kangwon National University";
const APP_VERSION: &str = "0.1.0";
const APP_EXE_NAME: &str = "knu_diagnosis.exe";
const TRAY_WINDOW_CLASS: PCWSTR = w!("KnuDiagnosisTrayWindowClass");
const UNINSTALL_REG_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\KNUDiagnosis";
const RUN_REG_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static APP_EXE_BYTES: &[u8] = include_bytes!("../../../target/release/knu_diagnosis.exe");
static ICON_BYTES: &[u8] = include_bytes!("../../../assets/knu_diagnosis.ico");
static LICENSE_BYTES: &[u8] = include_bytes!("../../../LICENSE");
static README_BYTES: &[u8] = include_bytes!("../../../README.md");

const IDC_DIR_EDIT: isize = 101;
const IDC_BROWSE_BTN: isize = 102;
const IDC_CHK_STARTMENU: isize = 103;
const IDC_CHK_DESKTOP: isize = 104;
const IDC_CHK_STARTUP: isize = 105;
const IDC_CHK_LAUNCH: isize = 106;
const IDC_STATUS_LBL: isize = 107;
const IDC_INSTALL_BTN: isize = 108;
const IDC_CANCEL_BTN: isize = 109;

struct SetupState {
    hwnd_dir: HWND,
    hwnd_startmenu: HWND,
    hwnd_desktop: HWND,
    hwnd_startup: HWND,
    hwnd_launch: HWND,
    hwnd_status: HWND,
}

fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

unsafe fn apply_gui_font(hwnd: HWND) {
    let font = GetStockObject(DEFAULT_GUI_FONT);
    let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
}

unsafe fn create_child(
    parent: HWND,
    hinstance: HINSTANCE,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: isize,
) -> Option<HWND> {
    let wide = to_wide_null(text);
    let hwnd = CreateWindowExW(
        ex_style,
        class,
        PCWSTR(wide.as_ptr()),
        style,
        x,
        y,
        w,
        h,
        parent,
        HMENU(id as *mut _),
        hinstance,
        None,
    )
    .ok()?;
    apply_gui_font(hwnd);
    Some(hwnd)
}

unsafe fn read_control_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let copied = GetWindowTextW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..copied as usize])
}

unsafe fn is_checked(hwnd: HWND) -> bool {
    SendMessageW(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == BST_CHECKED.0 as isize
}

unsafe fn load_embedded_icon(cx: i32, cy: i32) -> HICON {
    if let Ok(hmod) = GetModuleHandleW(None) {
        let hinst: HINSTANCE = hmod.into();
        if let Ok(handle) = LoadImageW(
            hinst,
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR | LR_SHARED,
        ) {
            if !handle.is_invalid() {
                return HICON(handle.0);
            }
        }
    }
    LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
}

fn default_install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
    base.join("Programs").join("KNUDiagnosis")
}

unsafe fn special_folder_path(csidl: u32) -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    if SHGetFolderPathW(None, csidl as i32, None, 0, &mut buf).is_ok() {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if end > 0 {
            return Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..end])));
        }
    }
    None
}

unsafe fn browse_for_folder(owner: HWND) -> Option<PathBuf> {
    let title = to_wide_null("Select installation folder for KNU Diagnosis:");
    let bi = BROWSEINFOW {
        hwndOwner: owner,
        lpszTitle: PCWSTR(title.as_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        ..Default::default()
    };
    let pidl = SHBrowseForFolderW(&bi);
    if pidl.is_null() {
        return None;
    }
    let mut path_buf = [0u16; 260];
    let ok = SHGetPathFromIDListW(pidl, &mut path_buf).as_bool();
    CoTaskMemFree(Some(pidl as *const _));
    if ok {
        let end = path_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(path_buf.len());
        if end > 0 {
            return Some(PathBuf::from(std::ffi::OsString::from_wide(
                &path_buf[..end],
            )));
        }
    }
    None
}

unsafe fn stop_running_instance() {
    if let Ok(tray_hwnd) = FindWindowW(TRAY_WINDOW_CLASS, None) {
        if !tray_hwnd.0.is_null() {
            let _ = PostMessageW(tray_hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(tray_hwnd, WM_DESTROY, WPARAM(0), LPARAM(0));
            for _ in 0..20 {
                thread::sleep(Duration::from_millis(100));
                if FindWindowW(TRAY_WINDOW_CLASS, None).is_err() {
                    break;
                }
            }
        }
    }
    let _ = Command::new("taskkill.exe")
        .args(["/F", "/IM", APP_EXE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    thread::sleep(Duration::from_millis(200));
}

unsafe fn create_shortcut(
    lnk_path: &Path,
    target_exe: &Path,
    working_dir: &Path,
    arguments: &str,
    description: &str,
    icon_exe: &Path,
) -> Result<(), String> {
    if let Some(parent) = lnk_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create shortcut dir: {e}"))?;
    }

    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
        .map_err(|e| format!("CoCreateInstance(ShellLink): {e}"))?;

    let w_target = to_wide_null(&target_exe.to_string_lossy());
    let w_dir = to_wide_null(&working_dir.to_string_lossy());
    let w_args = to_wide_null(arguments);
    let w_desc = to_wide_null(description);
    let w_icon = to_wide_null(&icon_exe.to_string_lossy());

    link.SetPath(PCWSTR(w_target.as_ptr()))
        .map_err(|e| format!("SetPath: {e}"))?;
    link.SetWorkingDirectory(PCWSTR(w_dir.as_ptr()))
        .map_err(|e| format!("SetWorkingDirectory: {e}"))?;
    if !arguments.is_empty() {
        link.SetArguments(PCWSTR(w_args.as_ptr()))
            .map_err(|e| format!("SetArguments: {e}"))?;
    }
    link.SetDescription(PCWSTR(w_desc.as_ptr()))
        .map_err(|e| format!("SetDescription: {e}"))?;
    link.SetIconLocation(PCWSTR(w_icon.as_ptr()), 0)
        .map_err(|e| format!("SetIconLocation: {e}"))?;

    let persist: IPersistFile = link
        .cast()
        .map_err(|e| format!("query IPersistFile: {e}"))?;
    let w_lnk = to_wide_null(&lnk_path.to_string_lossy());
    persist
        .Save(PCWSTR(w_lnk.as_ptr()), true)
        .map_err(|e| format!("IPersistFile::Save: {e}"))?;
    Ok(())
}

unsafe fn reg_set_sz(hkey: HKEY, name: &str, value: &str) -> Result<(), String> {
    let w_name = to_wide_null(name);
    let w_val = to_wide_null(value);
    let bytes = std::slice::from_raw_parts(w_val.as_ptr() as *const u8, w_val.len() * 2);
    let status = RegSetValueExW(hkey, PCWSTR(w_name.as_ptr()), 0, REG_SZ, Some(bytes));
    if status.is_err() {
        return Err(format!("RegSetValueExW({name}) failed: {:?}", status));
    }
    Ok(())
}

unsafe fn reg_set_dword(hkey: HKEY, name: &str, value: u32) -> Result<(), String> {
    let w_name = to_wide_null(name);
    let bytes = value.to_le_bytes();
    let status = RegSetValueExW(hkey, PCWSTR(w_name.as_ptr()), 0, REG_DWORD, Some(&bytes));
    if status.is_err() {
        return Err(format!("RegSetValueExW({name}) failed: {:?}", status));
    }
    Ok(())
}

unsafe fn write_uninstall_registry(install_dir: &Path) -> Result<(), String> {
    let subkey = to_wide_null(UNINSTALL_REG_SUBKEY);
    let mut hkey = HKEY::default();
    let status = RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), &mut hkey);
    if status.is_err() {
        return Err(format!("RegCreateKeyW failed: {:?}", status));
    }

    let exe_path = install_dir.join(APP_EXE_NAME);
    let uninst_path = install_dir.join("Uninstall.exe");
    let size_kb = ((APP_EXE_BYTES.len() + ICON_BYTES.len()) / 1024).max(1) as u32;

    let res = (|| {
        reg_set_sz(hkey, "DisplayName", APP_NAME)?;
        reg_set_sz(hkey, "Publisher", APP_PROVIDER)?;
        reg_set_sz(hkey, "DisplayVersion", APP_VERSION)?;
        reg_set_sz(hkey, "DisplayIcon", &format!("{},0", exe_path.display()))?;
        reg_set_sz(hkey, "InstallLocation", &install_dir.to_string_lossy())?;
        reg_set_sz(
            hkey,
            "UninstallString",
            &format!("\"{}\" --uninstall", uninst_path.display()),
        )?;
        reg_set_sz(hkey, "HelpLink", "https://www.kangwon.ac.kr")?;
        reg_set_dword(hkey, "EstimatedSize", size_kb)?;
        reg_set_dword(hkey, "NoModify", 1)?;
        reg_set_dword(hkey, "NoRepair", 1)?;
        Ok(())
    })();

    let _ = RegCloseKey(hkey);
    res
}

unsafe fn set_windows_startup(enabled: bool, exe_path: &Path) -> Result<(), String> {
    let subkey = to_wide_null(RUN_REG_SUBKEY);
    if enabled {
        let mut hkey = HKEY::default();
        let status = RegCreateKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), &mut hkey);
        if status.is_err() {
            return Err(format!("RegCreateKeyW(Run) failed: {:?}", status));
        }
        let res = reg_set_sz(hkey, APP_NAME, &format!("\"{}\"", exe_path.display()));
        let _ = RegCloseKey(hkey);
        res
    } else {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
        .is_ok()
        {
            let w_val = to_wide_null(APP_NAME);
            let _ = RegDeleteValueW(hkey, PCWSTR(w_val.as_ptr()));
            let _ = RegCloseKey(hkey);
        }
        Ok(())
    }
}

unsafe fn perform_install(
    hwnd: HWND,
    st: &SetupState,
    install_dir: &Path,
    make_start_menu: bool,
    make_desktop: bool,
    enable_startup: bool,
    launch_after: bool,
) -> Result<(), String> {
    let set_status = |msg: &str| {
        let wide = to_wide_null(msg);
        let _ = SetWindowTextW(st.hwnd_status, PCWSTR(wide.as_ptr()));
    };

    set_status("Stopping any running instance...");
    stop_running_instance();

    set_status("Extracting KNU Diagnosis files...");
    fs::create_dir_all(install_dir)
        .map_err(|e| format!("Failed to create directory {}: {e}", install_dir.display()))?;

    let exe_path = install_dir.join(APP_EXE_NAME);
    let ico_path = install_dir.join("knu_diagnosis.ico");
    let license_path = install_dir.join("LICENSE");
    let readme_path = install_dir.join("README.md");
    let uninst_path = install_dir.join("Uninstall.exe");

    fs::write(&exe_path, APP_EXE_BYTES)
        .map_err(|e| format!("Failed to write {}: {e}", exe_path.display()))?;
    fs::write(&ico_path, ICON_BYTES)
        .map_err(|e| format!("Failed to write {}: {e}", ico_path.display()))?;
    fs::write(&license_path, LICENSE_BYTES)
        .map_err(|e| format!("Failed to write {}: {e}", license_path.display()))?;
    fs::write(&readme_path, README_BYTES)
        .map_err(|e| format!("Failed to write {}: {e}", readme_path.display()))?;

    if let Ok(self_exe) = std::env::current_exe() {
        if self_exe != uninst_path {
            fs::copy(&self_exe, &uninst_path)
                .map_err(|e| format!("Failed to write Uninstaller: {e}"))?;
        }
    }

    set_status("Creating shortcuts...");
    if make_start_menu {
        if let Some(programs_dir) = special_folder_path(CSIDL_PROGRAMS) {
            let sm_dir = programs_dir.join(APP_NAME);
            create_shortcut(
                &sm_dir.join("KNU Diagnosis.lnk"),
                &exe_path,
                install_dir,
                "",
                "KNU Diagnosis — Kangwon National University",
                &exe_path,
            )?;
            create_shortcut(
                &sm_dir.join("Uninstall KNU Diagnosis.lnk"),
                &uninst_path,
                install_dir,
                "--uninstall",
                "Uninstall KNU Diagnosis",
                &uninst_path,
            )?;
        }
    }

    if make_desktop {
        if let Some(desktop_dir) = special_folder_path(CSIDL_DESKTOPDIRECTORY) {
            create_shortcut(
                &desktop_dir.join("KNU Diagnosis.lnk"),
                &exe_path,
                install_dir,
                "",
                "KNU Diagnosis — Kangwon National University",
                &exe_path,
            )?;
        }
    }

    set_status("Registering application metadata...");
    write_uninstall_registry(install_dir)?;
    set_windows_startup(enable_startup, &exe_path)?;

    if launch_after {
        set_status("Launching KNU Diagnosis...");
        let _ = Command::new(&exe_path).current_dir(install_dir).spawn();
    }

    set_status("Installation complete.");
    let done_msg = to_wide_null(&format!(
        "{APP_NAME} ({APP_PROVIDER}) has been installed to:\n{}",
        install_dir.display()
    ));
    let _ = MessageBoxW(
        hwnd,
        PCWSTR(done_msg.as_ptr()),
        w!("KNU Diagnosis Setup"),
        MB_OK | MB_ICONINFORMATION,
    );
    Ok(())
}

unsafe fn run_uninstall_flow() {
    let answer = MessageBoxW(
        None,
        w!("Uninstall KNU Diagnosis (Kangwon National University)?\n\nSaved settings and history in AppData will be preserved."),
        w!("Uninstall KNU Diagnosis"),
        MB_YESNO | MB_ICONQUESTION,
    );
    if answer != IDYES {
        return;
    }

    stop_running_instance();

    if let Some(programs_dir) = special_folder_path(CSIDL_PROGRAMS) {
        let sm_dir = programs_dir.join(APP_NAME);
        let _ = fs::remove_dir_all(&sm_dir);
    }
    if let Some(desktop_dir) = special_folder_path(CSIDL_DESKTOPDIRECTORY) {
        let _ = fs::remove_file(desktop_dir.join("KNU Diagnosis.lnk"));
    }

    let _ = set_windows_startup(false, Path::new(""));
    let subkey = to_wide_null(UNINSTALL_REG_SUBKEY);
    let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()));

    if let Ok(self_exe) = std::env::current_exe() {
        if let Some(install_dir) = self_exe.parent() {
            for rel in [APP_EXE_NAME, "knu_diagnosis.ico", "LICENSE", "README.md"] {
                let _ = fs::remove_file(install_dir.join(rel));
            }
            let script = format!(
                "ping -n 2 127.0.0.1 >nul & del /f /q \"{}\" & rmdir \"{}\"",
                self_exe.display(),
                install_dir.display()
            );
            let _ = Command::new("cmd.exe")
                .args(["/C", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
    }

    let _ = MessageBoxW(
        None,
        w!("KNU Diagnosis has been uninstalled."),
        w!("Uninstall KNU Diagnosis"),
        MB_OK | MB_ICONINFORMATION,
    );
}

unsafe extern "system" fn setup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let cs = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SetupState;

    match msg {
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let st = &*state_ptr;
                let id = (wparam.0 & 0xFFFF) as isize;
                if id == IDC_BROWSE_BTN {
                    if let Some(chosen) = browse_for_folder(hwnd) {
                        let target = if chosen
                            .file_name()
                            .map(|n| n.to_string_lossy().eq_ignore_ascii_case("KNUDiagnosis"))
                            .unwrap_or(false)
                        {
                            chosen
                        } else {
                            chosen.join("KNUDiagnosis")
                        };
                        let w_dir = to_wide_null(&target.to_string_lossy());
                        let _ = SetWindowTextW(st.hwnd_dir, PCWSTR(w_dir.as_ptr()));
                    }
                } else if id == IDC_INSTALL_BTN {
                    let dir_text = read_control_text(st.hwnd_dir);
                    let trimmed = dir_text.trim();
                    if trimmed.is_empty() {
                        let _ = MessageBoxW(
                            hwnd,
                            w!("Please select a valid installation folder."),
                            w!("KNU Diagnosis Setup"),
                            MB_OK | MB_ICONERROR,
                        );
                        return LRESULT(0);
                    }

                    let make_sm = is_checked(st.hwnd_startmenu);
                    let make_desk = is_checked(st.hwnd_desktop);
                    let startup = is_checked(st.hwnd_startup);
                    let launch = is_checked(st.hwnd_launch);

                    match perform_install(
                        hwnd,
                        st,
                        Path::new(trimmed),
                        make_sm,
                        make_desk,
                        startup,
                        launch,
                    ) {
                        Ok(()) => {
                            let _ = DestroyWindow(hwnd);
                        }
                        Err(err) => {
                            let w_err = to_wide_null(&err);
                            let _ = MessageBoxW(
                                hwnd,
                                PCWSTR(w_err.as_ptr()),
                                w!("Installation Failed"),
                                MB_OK | MB_ICONERROR,
                            );
                        }
                    }
                } else if id == IDC_CANCEL_BTN {
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                let _ = Box::from_raw(state_ptr);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let is_uninstall_arg = std::env::args().any(|a| a.eq_ignore_ascii_case("--uninstall"));
        let is_uninstall_exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .map(|name| name.eq_ignore_ascii_case("Uninstall.exe"))
            .unwrap_or(false);

        if is_uninstall_arg || is_uninstall_exe {
            run_uninstall_flow();
            CoUninitialize();
            return;
        }

        let Ok(hmodule) = GetModuleHandleW(None) else {
            CoUninitialize();
            return;
        };
        let hinst: HINSTANCE = hmodule.into();
        let class_name = w!("KnuDiagnosisInstallerWindowClass");

        let wc = WNDCLASSW {
            lpfnWndProc: Some(setup_wnd_proc),
            hInstance: hinst,
            lpszClassName: class_name,
            hIcon: load_embedded_icon(GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON)),
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as *mut _),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let state = Box::new(SetupState {
            hwnd_dir: HWND::default(),
            hwnd_startmenu: HWND::default(),
            hwnd_desktop: HWND::default(),
            hwnd_startup: HWND::default(),
            hwnd_launch: HWND::default(),
            hwnd_status: HWND::default(),
        });
        let state_ptr = Box::into_raw(state);

        let Ok(hwnd) = CreateWindowExW(
            Default::default(),
            class_name,
            w!("KNU Diagnosis v0.1.0 Setup — Kangwon National University"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            540,
            372,
            None,
            None,
            hinst,
            Some(state_ptr as *const _),
        ) else {
            let _ = Box::from_raw(state_ptr);
            CoUninitialize();
            return;
        };

        // Logo icon preview in header
        if let Some(logo_ctl) = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            "",
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_ICON),
            Default::default(),
            18,
            16,
            64,
            64,
            0,
        ) {
            let big_icon = load_embedded_icon(64, 64);
            let _ = SendMessageW(logo_ctl, STM_SETICON, WPARAM(big_icon.0 as usize), LPARAM(0));
        }
        let small_icon = load_embedded_icon(
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
        );
        let _ = SendMessageW(hwnd, 0x0080 /* WM_SETICON */, WPARAM(0), LPARAM(small_icon.0 as isize));

        let _ = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            "KNU Diagnosis v0.1.0",
            WS_CHILD | WS_VISIBLE,
            Default::default(),
            96,
            18,
            400,
            22,
            0,
        );
        let _ = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            "Provider: Kangwon National University",
            WS_CHILD | WS_VISIBLE,
            Default::default(),
            96,
            40,
            400,
            20,
            0,
        );
        let _ = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            "Install KNU Diagnosis on this Windows computer (user-level install, no admin required).",
            WS_CHILD | WS_VISIBLE,
            Default::default(),
            96,
            62,
            420,
            20,
            0,
        );

        let _ = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            "Installation Folder:",
            WS_CHILD | WS_VISIBLE,
            Default::default(),
            18,
            98,
            160,
            18,
            0,
        );
        let default_dir = default_install_dir().to_string_lossy().into_owned();
        let hwnd_dir = create_child(
            hwnd,
            hinst,
            w!("EDIT"),
            &default_dir,
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32),
            Default::default(),
            18,
            118,
            404,
            24,
            IDC_DIR_EDIT,
        )
        .unwrap_or_default();
        let _ = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Browse...",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            Default::default(),
            430,
            117,
            82,
            26,
            IDC_BROWSE_BTN,
        );

        let chk_style =
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_AUTOCHECKBOX as u32);
        let hwnd_startmenu = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Create Start Menu shortcut",
            chk_style,
            Default::default(),
            18,
            156,
            320,
            22,
            IDC_CHK_STARTMENU,
        )
        .unwrap_or_default();
        let hwnd_desktop = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Create Desktop shortcut",
            chk_style,
            Default::default(),
            18,
            182,
            320,
            22,
            IDC_CHK_DESKTOP,
        )
        .unwrap_or_default();
        let hwnd_startup = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Start KNU Diagnosis automatically when Windows signs in",
            chk_style,
            Default::default(),
            18,
            208,
            420,
            22,
            IDC_CHK_STARTUP,
        )
        .unwrap_or_default();
        let hwnd_launch = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Launch KNU Diagnosis after installation",
            chk_style,
            Default::default(),
            18,
            234,
            360,
            22,
            IDC_CHK_LAUNCH,
        )
        .unwrap_or_default();

        let _ = SendMessageW(hwnd_startmenu, BM_SETCHECK, WPARAM(BST_CHECKED.0 as usize), LPARAM(0));
        let _ = SendMessageW(hwnd_desktop, BM_SETCHECK, WPARAM(BST_CHECKED.0 as usize), LPARAM(0));
        let _ = SendMessageW(hwnd_startup, BM_SETCHECK, WPARAM(BST_UNCHECKED.0 as usize), LPARAM(0));
        let _ = SendMessageW(hwnd_launch, BM_SETCHECK, WPARAM(BST_CHECKED.0 as usize), LPARAM(0));

        let hwnd_status = create_child(
            hwnd,
            hinst,
            w!("STATIC"),
            "Ready to install.",
            WS_CHILD | WS_VISIBLE,
            Default::default(),
            18,
            274,
            300,
            20,
            IDC_STATUS_LBL,
        )
        .unwrap_or_default();

        let _ = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Install",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            Default::default(),
            324,
            286,
            90,
            28,
            IDC_INSTALL_BTN,
        );
        let _ = create_child(
            hwnd,
            hinst,
            w!("BUTTON"),
            "Cancel",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            Default::default(),
            422,
            286,
            90,
            28,
            IDC_CANCEL_BTN,
        );

        if let Some(st) = (state_ptr as *mut SetupState).as_mut() {
            st.hwnd_dir = hwnd_dir;
            st.hwnd_startmenu = hwnd_startmenu;
            st.hwnd_desktop = hwnd_desktop;
            st.hwnd_startup = hwnd_startup;
            st.hwnd_launch = hwnd_launch;
            st.hwnd_status = hwnd_status;
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        CoUninitialize();
    }
}
