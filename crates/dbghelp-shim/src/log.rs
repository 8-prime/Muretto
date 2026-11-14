//! An optional trace of what the patch asks for, written to the file named
//! by `MURETTO_DBGHELP_LOG`.

use std::{
    fmt::Arguments,
    fs::File,
    io::Write,
    sync::{Mutex, OnceLock},
};

pub fn line(message: Arguments) {
    static FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("MURETTO_DBGHELP_LOG")?;
        File::create(path).ok().map(Mutex::new)
    });
    if let Some(file) = file
        && let Ok(mut file) = file.lock()
    {
        let _ = writeln!(file, "{message}");
    }
}
