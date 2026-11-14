//! The symbols of a module's PDB that Wine's dbghelp leaves out of an
//! enumeration: the public symbols, and the functions of compilands it has
//! not loaded yet.

use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use pdb::FallibleIterator;

use crate::{
    log,
    real::{self, Module},
};

pub const SYM_TAG_FUNCTION: u32 = 5;
pub const SYM_TAG_PUBLIC_SYMBOL: u32 = 10;

pub struct Symbol {
    /// The name as the PDB stores it: decorated for public symbols
    /// (`?render@CarLabel@@QEAAXXZ`), plain for functions (`CarLabel::render`).
    pub name: String,
    /// The plain form, shown with `SYMOPT_UNDNAME`.
    pub undecorated: String,
    pub rva: u32,
    pub size: u32,
    pub tag: u32,
    pub code: bool,
}

impl Symbol {
    pub fn name(&self, undecorate: bool) -> &str {
        if undecorate {
            &self.undecorated
        } else {
            &self.name
        }
    }
}

pub struct Table {
    list: Vec<Symbol>,
    by_name: HashMap<String, usize>,
}

impl Table {
    pub fn iter(&self) -> impl Iterator<Item = &Symbol> {
        self.list.iter()
    }

    /// Looks a symbol up by either form of its name.
    pub fn find(&self, name: &str) -> Option<&Symbol> {
        self.by_name.get(name).map(|&index| &self.list[index])
    }
}

/// The symbols of `module`, read once and kept. `None` when its PDB cannot be
/// found or does not belong to the loaded image.
pub fn for_module(module: &Module) -> Option<Arc<Table>> {
    static TABLES: Mutex<Option<HashMap<PathBuf, Option<Arc<Table>>>>> = Mutex::new(None);
    let mut tables = TABLES.lock().ok()?;
    tables
        .get_or_insert_with(HashMap::new)
        .entry(module.image.clone())
        .or_insert_with(|| {
            let table = load(module);
            match &table {
                Ok(table) => log::line(format_args!(
                    "{}: {} symbols read from the PDB",
                    module.name,
                    table.list.len()
                )),
                Err(reason) => {
                    log::line(format_args!("{}: no symbols added, {reason}", module.name))
                }
            }
            table.ok().map(Arc::new)
        })
        .clone()
}

fn load(module: &Module) -> Result<Table, String> {
    let record = unsafe { codeview(module.base) }.ok_or("the image names no PDB")?;
    let beside_image = module.image.with_extension("pdb");
    let named = PathBuf::from(&record.path);
    let path = [beside_image, named]
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| format!("{} not found", record.path))?;

    let failed = |e: pdb::Error| format!("{}: {e}", path.display());
    let file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut pdb = pdb::PDB::open(file).map_err(failed)?;
    let info = pdb.pdb_information().map_err(failed)?;
    if info.guid.to_bytes_le() != record.guid {
        return Err(format!("{} belongs to a different build", path.display()));
    }
    let addresses = pdb.address_map().map_err(failed)?;

    let mut list = Vec::new();
    let globals = pdb.global_symbols().map_err(failed)?;
    let mut iter = globals.iter();
    while let Ok(Some(symbol)) = iter.next() {
        let Ok(pdb::SymbolData::Public(public)) = symbol.parse() else {
            continue;
        };
        let Some(rva) = public.offset.to_rva(&addresses) else {
            continue;
        };
        let name = public.name.to_string().into_owned();
        let undecorated = match name.starts_with('?') {
            true => real::undecorate(&name).unwrap_or_else(|| name.clone()),
            false => name.clone(),
        };
        list.push(Symbol {
            name,
            undecorated,
            rva: rva.0,
            size: 0,
            tag: SYM_TAG_PUBLIC_SYMBOL,
            code: public.code || public.function,
        });
    }

    let debug = pdb.debug_information().map_err(failed)?;
    let mut compilands = debug.modules().map_err(failed)?;
    while let Ok(Some(compiland)) = compilands.next() {
        let Ok(Some(info)) = pdb.module_info(&compiland) else {
            continue;
        };
        let Ok(mut symbols) = info.symbols() else {
            continue;
        };
        while let Ok(Some(symbol)) = symbols.next() {
            let Ok(pdb::SymbolData::Procedure(procedure)) = symbol.parse() else {
                continue;
            };
            let Some(rva) = procedure.offset.to_rva(&addresses) else {
                continue;
            };
            let name = procedure.name.to_string().into_owned();
            list.push(Symbol {
                undecorated: name.clone(),
                name,
                rva: rva.0,
                size: procedure.len,
                tag: SYM_TAG_FUNCTION,
                code: true,
            });
        }
    }

    let mut by_name = HashMap::with_capacity(list.len() * 2);
    for (index, symbol) in list.iter().enumerate() {
        by_name.entry(symbol.name.clone()).or_insert(index);
        by_name.entry(symbol.undecorated.clone()).or_insert(index);
    }
    Ok(Table { list, by_name })
}

/// The `RSDS` debug record of an image: which PDB it was built with.
struct CodeView {
    guid: [u8; 16],
    path: String,
}

/// Reads the CodeView record from the headers of the image mapped at `base`.
unsafe fn codeview(base: u64) -> Option<CodeView> {
    const IMAGE_DEBUG_TYPE_CODEVIEW: u32 = 2;
    let image = base as *const u8;
    let read_u16 = |offset: usize| unsafe { (image.add(offset) as *const u16).read_unaligned() };
    let read_u32 = |offset: usize| unsafe { (image.add(offset) as *const u32).read_unaligned() };

    if read_u16(0) != 0x5a4d {
        return None;
    }
    let nt = read_u32(0x3c) as usize;
    if read_u32(nt) != 0x0000_4550 {
        return None;
    }
    let optional = nt + 24;
    // PE32+ keeps its data directories 112 bytes into the optional header;
    // the debug directory is entry 6.
    if read_u16(optional) != 0x20b {
        return None;
    }
    let directory = optional + 112 + 6 * 8;
    let (start, size) = (
        read_u32(directory) as usize,
        read_u32(directory + 4) as usize,
    );
    for entry in (start..start + size).step_by(28) {
        if read_u32(entry + 12) != IMAGE_DEBUG_TYPE_CODEVIEW {
            continue;
        }
        let data = read_u32(entry + 20) as usize;
        if read_u32(data) != u32::from_le_bytes(*b"RSDS") {
            continue;
        }
        let mut guid = [0u8; 16];
        unsafe { std::ptr::copy_nonoverlapping(image.add(data + 4), guid.as_mut_ptr(), 16) };
        let path = unsafe { std::ffi::CStr::from_ptr(image.add(data + 24).cast()) };
        return Some(CodeView {
            guid,
            path: path.to_string_lossy().into_owned(),
        });
    }
    None
}
