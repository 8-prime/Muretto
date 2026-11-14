//! A `dbghelp.dll` for Custom Shaders Patch running under Wine.
//!
//! The patch finds the game's functions and globals by listing every symbol
//! of `acs.exe` with `SymEnumSymbols`, which reads them from `acs.pdb`.
//! Microsoft's dbghelp lists all of them. Wine's leaves out the PDB's public
//! symbols and the functions of compilands it has not loaded yet, so the
//! patch gives up with "Failed to patch Assetto Corsa: Can't find ...".
//!
//! Placed beside `acs.exe` with `WINEDLLOVERRIDES=dbghelp=n,b`, this library
//! is loaded instead of Wine's. Every function forwards to Wine's dbghelp;
//! `SymEnumSymbols` and `SymFromName` also report the symbols Wine misses,
//! read from the PDB the way Windows would report them.
//!
//! Set `MURETTO_DBGHELP_LOG` to a Windows path to log what the patch asks for.

#![cfg(windows)]
// The exports are dbghelp's own API, whose contracts are Microsoft's to document.
#![allow(clippy::missing_safety_doc)]

mod log;
mod real;
mod symbols;

use std::{
    collections::HashSet,
    ffi::{CStr, c_char, c_void},
    sync::OnceLock,
};

use symbols::Symbol;

/// `SYMBOL_INFO` from `dbghelp.h`, without the trailing name buffer.
#[repr(C)]
pub struct SymbolInfo {
    size_of_struct: u32,
    type_index: u32,
    reserved: [u64; 2],
    index: u32,
    size: u32,
    mod_base: u64,
    flags: u32,
    value: u64,
    address: u64,
    register: u32,
    scope: u32,
    tag: u32,
    name_len: u32,
    max_name_len: u32,
    name: [u8; 1],
}

/// `sizeof(SYMBOL_INFO)`, which callers put in `SizeOfStruct`.
const SYMBOL_INFO_SIZE: u32 = 88;
const _: () = assert!(size_of::<SymbolInfo>() == SYMBOL_INFO_SIZE as usize);

const SYMFLAG_PUBLIC_CODE: u32 = 0x0040_0000;
const SYMOPT_UNDNAME: u32 = 0x2;

type EnumCallback = unsafe extern "system" fn(*const SymbolInfo, u32, *mut c_void) -> i32;

/// Lets Muretto tell its own copy from any other `dbghelp.dll` in the game
/// folder. Keep in step with `MARKER` in the core crate's `csp` module.
#[used]
static MARKER: [u8; 20] = *b"muretto-dbghelp-shim";

/// Functions the patch imports that need no change: forwarded as they are.
/// Every argument and return value fits a register, which is all the x64
/// calling convention needs to pass them through untouched.
macro_rules! forward {
    ($($name:ident($($arg:ident),*);)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg: usize),*) -> usize {
            static TARGET: OnceLock<usize> = OnceLock::new();
            let target = *TARGET.get_or_init(|| real::proc(concat!(stringify!($name), "\0")));
            if target == 0 {
                log::line(format_args!("{} is missing from the system dbghelp", stringify!($name)));
                return 0;
            }
            let target: unsafe extern "system" fn($(forward!(@usize $arg)),*) -> usize =
                unsafe { std::mem::transmute(target) };
            unsafe { target($($arg),*) }
        }
    )*};
    (@usize $arg:ident) => { usize };
}

forward! {
    StackWalk64(a, b, c, d, e, f, g, h, i);
    SymFunctionTableAccess64(a, b);
    SymGetModuleBase64(a, b);
    SymCleanup(a);
    SymEnumerateModulesW64(a, b, c);
    SymEnumerateModules64(a, b, c);
    SymSetSearchPathW(a, b);
    SymGetSearchPathW(a, b, c);
    SymInitialize(a, b, c);
    SymGetOptions();
    SymGetModuleInfoW64(a, b, c);
    SymGetModuleInfo64(a, b, c);
    SymFromAddr(a, b, c, d);
    SymFromAddrW(a, b, c, d);
    SymGetLineFromAddr64(a, b, c, d);
    SymGetLineFromAddrW64(a, b, c, d);
    SymRegisterCallbackW64(a, b, c);
    SymRegisterCallback64(a, b, c);
    SymRefreshModuleList(a);
    SymLoadModuleEx(a, b, c, d, e, f, g, h);
    SymLoadModuleExW(a, b, c, d, e, f, g, h);
    SymLoadModule64(a, b, c, d, e, f);
    SymUnloadModule64(a, b);
    SymMatchStringA(a, b, c);
    UnDecorateSymbolName(a, b, c, d);
    MiniDumpWriteDump(a, b, c, d, e, f, g);
    ImagehlpApiVersion();
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymInitializeW(
    process: usize,
    search: *const u16,
    invade: i32,
) -> i32 {
    let ok = unsafe { real::sym_initialize_w(process, search, invade) };
    log::line(format_args!("SymInitializeW(invade {invade}) -> {ok}"));
    ok
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymSetOptions(options: u32) -> u32 {
    log::line(format_args!("SymSetOptions({options:#x})"));
    unsafe { real::sym_set_options(options) }
}

/// What the wrapped callback needs to pass symbols on, to remember which
/// names it has seen, and to notice when the caller asks to stop.
struct Enumeration {
    callback: EnumCallback,
    context: *mut c_void,
    seen: HashSet<Vec<u8>>,
    stopped: bool,
}

unsafe extern "system" fn relay(symbol: *const SymbolInfo, size: u32, context: *mut c_void) -> i32 {
    let state = unsafe { &mut *(context as *mut Enumeration) };
    let name = unsafe {
        let len = (*symbol).name_len.min((*symbol).max_name_len) as usize;
        std::slice::from_raw_parts(std::ptr::addr_of!((*symbol).name).cast::<u8>(), len)
    };
    state.seen.insert(name.to_vec());
    let keep_going = unsafe { (state.callback)(symbol, size, state.context) };
    if keep_going == 0 {
        state.stopped = true;
    }
    keep_going
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymEnumSymbols(
    process: usize,
    base: u64,
    mask: *const c_char,
    callback: Option<EnumCallback>,
    context: *mut c_void,
) -> i32 {
    let Some(callback) = callback else {
        return unsafe { real::sym_enum_symbols(process, base, mask, None, context) };
    };
    let mask_text = (!mask.is_null()).then(|| {
        unsafe { CStr::from_ptr(mask) }
            .to_string_lossy()
            .into_owned()
    });
    let mut state = Enumeration {
        callback,
        context,
        seen: HashSet::new(),
        stopped: false,
    };
    let ok = unsafe {
        real::sym_enum_symbols(
            process,
            base,
            mask,
            Some(relay),
            &mut state as *mut _ as *mut c_void,
        )
    };
    let from_wine = state.seen.len();

    let (module_mask, name_mask) = match mask_text.as_deref() {
        Some(text) => match text.split_once('!') {
            Some((module, name)) => (Some(module), name),
            None => (None, text),
        },
        None => (None, "*"),
    };
    let mut added = 0;
    if !state.stopped {
        let undecorate = unsafe { real::sym_get_options() } & SYMOPT_UNDNAME != 0;
        'modules: for module in real::modules(process, base, module_mask) {
            let Some(table) = symbols::for_module(&module) else {
                continue;
            };
            for symbol in table.iter() {
                let name = symbol.name(undecorate);
                if state.seen.contains(name.as_bytes()) || !real::matches(name, name_mask) {
                    continue;
                }
                added += 1;
                let mut info = SymbolBuffer::new(module.base, symbol, name);
                if unsafe { relay(info.as_ptr(), 0, &mut state as *mut _ as *mut c_void) } == 0 {
                    break 'modules;
                }
            }
        }
    }
    log::line(format_args!(
        "SymEnumSymbols({:?}, base {base:#x}) -> {ok}: {from_wine} names from Wine, {added} added",
        mask_text.as_deref().unwrap_or("")
    ));
    if added > 0 { 1 } else { ok }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SymFromName(
    process: usize,
    name: *const c_char,
    symbol: *mut SymbolInfo,
) -> i32 {
    if unsafe { real::sym_from_name(process, name, symbol) } != 0 {
        return 1;
    }
    let wanted = unsafe { CStr::from_ptr(name) }.to_string_lossy();
    let (module_mask, wanted_name) = match wanted.split_once('!') {
        Some((module, rest)) => (Some(module), rest),
        None => (None, &*wanted),
    };
    for module in real::modules(process, 0, module_mask) {
        let Some(table) = symbols::for_module(&module) else {
            continue;
        };
        if let Some(found) = table.find(wanted_name) {
            let capacity = unsafe { (*symbol).max_name_len } as usize;
            let shown = if found.name == wanted_name {
                &found.name
            } else {
                &found.undecorated
            };
            unsafe { SymbolBuffer::fill(symbol, module.base, found, shown, capacity) };
            log::line(format_args!("SymFromName({wanted}) found in the PDB"));
            return 1;
        }
    }
    log::line(format_args!("SymFromName({wanted}) not found"));
    0
}

/// A `SYMBOL_INFO` with room for its name.
struct SymbolBuffer {
    words: Vec<u64>,
}

impl SymbolBuffer {
    fn new(mod_base: u64, symbol: &Symbol, name: &str) -> Self {
        let bytes = SYMBOL_INFO_SIZE as usize + name.len() + 1;
        let mut words = vec![0u64; bytes.div_ceil(8)];
        unsafe {
            Self::fill(
                words.as_mut_ptr().cast(),
                mod_base,
                symbol,
                name,
                name.len() + 1,
            )
        };
        SymbolBuffer { words }
    }

    fn as_ptr(&mut self) -> *const SymbolInfo {
        self.words.as_ptr().cast()
    }

    /// Writes `symbol` into the structure at `info`, whose name buffer holds
    /// `capacity` bytes.
    unsafe fn fill(
        info: *mut SymbolInfo,
        mod_base: u64,
        symbol: &Symbol,
        name: &str,
        capacity: usize,
    ) {
        unsafe {
            (*info).size_of_struct = SYMBOL_INFO_SIZE;
            (*info).type_index = 0;
            (*info).index = 0;
            (*info).size = symbol.size;
            (*info).mod_base = mod_base;
            (*info).flags = match symbol.tag == symbols::SYM_TAG_PUBLIC_SYMBOL && symbol.code {
                true => SYMFLAG_PUBLIC_CODE,
                false => 0,
            };
            (*info).value = 0;
            (*info).address = mod_base + u64::from(symbol.rva);
            (*info).register = 0;
            (*info).scope = 0;
            (*info).tag = symbol.tag;
            (*info).name_len = name.len() as u32;
            if capacity == 0 {
                return;
            }
            let copied = name.len().min(capacity - 1);
            let target = std::ptr::addr_of_mut!((*info).name).cast::<u8>();
            std::ptr::copy_nonoverlapping(name.as_ptr(), target, copied);
            *target.add(copied) = 0;
        }
    }
}
