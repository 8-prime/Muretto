//! Wine's own dbghelp, which does all the work this library does not.

use std::{
    ffi::{CString, c_char, c_void},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    sync::OnceLock,
};

use crate::{EnumCallback, SymbolInfo, log};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(module: usize, name: *const c_char) -> usize;
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetModuleFileNameW(module: usize, buffer: *mut u16, size: u32) -> u32;
}

fn library() -> usize {
    static LIBRARY: OnceLock<usize> = OnceLock::new();
    *LIBRARY.get_or_init(|| {
        let mut dir = vec![0u16; 512];
        let len = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), dir.len() as u32) } as usize;
        dir.truncate(len);
        dir.extend("\\dbghelp.dll".encode_utf16());
        dir.push(0);
        let module = unsafe { LoadLibraryW(dir.as_ptr()) };
        log::line(format_args!(
            "loaded {} -> {module:#x}",
            String::from_utf16_lossy(&dir[..dir.len() - 1])
        ));
        module
    })
}

/// The address of `name` (nul-terminated) in the system dbghelp, or 0.
pub fn proc(name: &str) -> usize {
    debug_assert!(name.ends_with('\0'));
    match library() {
        0 => 0,
        module => unsafe { GetProcAddress(module, name.as_ptr().cast()) },
    }
}

macro_rules! typed {
    ($fn_name:ident = $symbol:literal: fn($($arg:ident: $ty:ty),*) -> $ret:ty, or $fallback:expr) => {
        pub unsafe fn $fn_name($($arg: $ty),*) -> $ret {
            static TARGET: OnceLock<usize> = OnceLock::new();
            let target = *TARGET.get_or_init(|| proc(concat!($symbol, "\0")));
            if target == 0 {
                return $fallback;
            }
            let target: unsafe extern "system" fn($($ty),*) -> $ret = unsafe { std::mem::transmute(target) };
            unsafe { target($($arg),*) }
        }
    };
}

typed!(sym_initialize_w = "SymInitializeW": fn(process: usize, search: *const u16, invade: i32) -> i32, or 0);
typed!(sym_set_options = "SymSetOptions": fn(options: u32) -> u32, or 0);
typed!(sym_get_options = "SymGetOptions": fn() -> u32, or 0);
typed!(sym_enum_symbols = "SymEnumSymbols":
    fn(process: usize, base: u64, mask: *const c_char, callback: Option<EnumCallback>, context: *mut c_void) -> i32, or 0);
typed!(sym_from_name = "SymFromName": fn(process: usize, name: *const c_char, symbol: *mut SymbolInfo) -> i32, or 0);
typed!(sym_match_string = "SymMatchStringA": fn(text: *const c_char, pattern: *const c_char, case: i32) -> i32, or 0);
typed!(undecorate_raw = "UnDecorateSymbolName": fn(name: *const c_char, out: *mut c_char, size: u32, flags: u32) -> u32, or 0);
typed!(enumerate_modules = "SymEnumerateModulesW64":
    fn(process: usize, callback: unsafe extern "system" fn(*const u16, u64, *mut c_void) -> i32, context: *mut c_void) -> i32, or 0);

/// Whether `name` matches a dbghelp wildcard mask such as `*` or `Car*`.
pub fn matches(name: &str, mask: &str) -> bool {
    if mask == "*" {
        return true;
    }
    let (Ok(name), Ok(mask)) = (CString::new(name), CString::new(mask)) else {
        return false;
    };
    unsafe { sym_match_string(name.as_ptr(), mask.as_ptr(), 0) != 0 }
}

/// The plain `Class::member` form of a decorated C++ name, as dbghelp shows it
/// with `SYMOPT_UNDNAME`.
pub fn undecorate(name: &str) -> Option<String> {
    const UNDNAME_NAME_ONLY: u32 = 0x1000;
    let name = CString::new(name).ok()?;
    let mut out = vec![0u8; 1024];
    let len = unsafe {
        undecorate_raw(
            name.as_ptr(),
            out.as_mut_ptr().cast(),
            out.len() as u32,
            UNDNAME_NAME_ONLY,
        )
    };
    (len > 0).then(|| String::from_utf8_lossy(&out[..len as usize]).into_owned())
}

/// A module loaded into the inspected process.
pub struct Module {
    pub base: u64,
    pub name: String,
    pub image: PathBuf,
}

/// The modules dbghelp knows for `process`: the one at `base` when it is not
/// zero, otherwise those whose name matches `mask` (all when there is none).
/// Only modules mapped into this process qualify: their headers are read
/// from memory.
pub fn modules(process: usize, base: u64, mask: Option<&str>) -> Vec<Module> {
    unsafe extern "system" fn collect(name: *const u16, base: u64, context: *mut c_void) -> i32 {
        let found = unsafe { &mut *(context as *mut Vec<(String, u64)>) };
        let len = (0..).take_while(|&i| unsafe { *name.add(i) } != 0).count();
        found.push((
            String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(name, len) }),
            base,
        ));
        1
    }
    let mut found: Vec<(String, u64)> = Vec::new();
    unsafe { enumerate_modules(process, collect, &mut found as *mut _ as *mut c_void) };
    found
        .into_iter()
        .filter(|(name, module_base)| match (base, mask) {
            (0, Some(mask)) => {
                mask == "*" || name.eq_ignore_ascii_case(mask) || matches(name, mask)
            }
            (0, None) => true,
            (base, _) => *module_base == base,
        })
        .filter_map(|(name, base)| {
            let mut path = vec![0u16; 1024];
            let len =
                unsafe { GetModuleFileNameW(base as usize, path.as_mut_ptr(), path.len() as u32) }
                    as usize;
            (len > 0).then(|| Module {
                base,
                name,
                image: std::ffi::OsString::from_wide(&path[..len]).into(),
            })
        })
        .collect()
}
